use dim_agent_capabilities::host::ModelProtocol;
use dim_agent_runtime::{Capability, KeysOf, capability_plugin};
use dim_app_server::managed_provider::{
    AccountSnapshot, CredentialLease, LeaseOwner, ManagedCatalog, ManagedError, ManagedFuture,
    ManagedProvider, ManagedProviderBinding, ManagedProviders, ManagedProvidersPlugin,
    ProviderDescriptor,
    management::{
        BrowserLogin, BrowserLoginBegin, DeviceLogin, LoginBegin, LoginState,
        ManagedBrowserLoginComplete, ManagedLoginAttempt, Usage,
    },
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, FiberState, TypeKey, TypedPlugin};
use std::sync::{
    Arc, Mutex, OnceLock, Weak,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Leases {
    issued: Mutex<Vec<String>>,
    released: Mutex<Vec<String>>,
}
impl LeaseOwner for Leases {
    fn release(&self, lease_id: &str) {
        self.released.lock().unwrap().push(lease_id.into());
    }
}

#[derive(Default)]
struct Fixture {
    leases: Arc<Leases>,
    entered: AtomicUsize,
    resume: Notify,
    quiesced: AtomicBool,
    /// The account recovery reports.
    recovered_generation: Mutex<u64>,
}
fn account(generation: u64) -> AccountSnapshot {
    AccountSnapshot {
        credential_epoch: "epoch".into(),
        generation,
        account: None,
    }
}
impl Fixture {
    fn lease(&self, generation: u64) -> CredentialLease {
        let mut issued = self.leases.issued.lock().unwrap();
        let id = format!("lease-{}", issued.len());
        issued.push(id.clone());
        let owner: Weak<dyn LeaseOwner> = Arc::downgrade(&self.leases) as Weak<Leases>;
        CredentialLease::new(
            account(generation),
            ModelProtocol::OpenaiCompatible,
            "http://localhost/v1".into(),
            Default::default(),
            "private-fixture-token".into(),
            id,
            owner,
        )
    }
    /// Issues a lease from owned work that waits for `resume`, as a provider
    /// owns its accepted operations.
    fn issue(self: &Arc<Self>, generation: u64) -> ManagedFuture<CredentialLease> {
        let owner = self.clone();
        let task = tokio::spawn(async move {
            owner.entered.fetch_add(1, Ordering::SeqCst);
            owner.resume.notified().await;
            Ok(owner.lease(generation))
        });
        Box::pin(async move { task.await.unwrap() })
    }
}
struct Provider(Arc<Fixture>);
fn unused<T: Send + 'static>() -> ManagedFuture<T> {
    Box::pin(async { Err(ManagedError::InvalidInput) })
}
impl ManagedProvider for Provider {
    fn providers(&self) -> Vec<ProviderDescriptor> {
        vec![ProviderDescriptor {
            provider_id: "fixture".into(),
            display_name: "Fixture".into(),
            login_methods: vec!["device".into()],
        }]
    }
    fn status(&self, _: &str) -> ManagedFuture<AccountSnapshot> {
        unused()
    }
    fn recover(&self, _: &str) -> ManagedFuture<AccountSnapshot> {
        unused()
    }
    fn begin_browser(&self, _: BrowserLoginBegin) -> ManagedFuture<BrowserLogin> {
        unused()
    }
    fn complete_browser(&self, _: ManagedBrowserLoginComplete) -> ManagedFuture<AccountSnapshot> {
        unused()
    }
    fn begin_device(&self, _: LoginBegin) -> ManagedFuture<DeviceLogin> {
        unused()
    }
    fn attempt_status(&self, _: &ManagedLoginAttempt) -> Result<LoginState, ManagedError> {
        Err(ManagedError::AttemptNotFound)
    }
    fn cancel_attempt(&self, _: ManagedLoginAttempt) -> ManagedFuture<()> {
        unused()
    }
    fn logout(&self, _: &str) -> ManagedFuture<AccountSnapshot> {
        unused()
    }
    fn usage(&self, _: &str) -> ManagedFuture<Usage> {
        unused()
    }
    fn catalog(&self, _: &str) -> ManagedFuture<ManagedCatalog> {
        unused()
    }
    fn credential(&self, _: &str, _: &str) -> ManagedFuture<CredentialLease> {
        self.0.issue(1)
    }
    fn recover_rejected(&self, _: &CredentialLease) -> ManagedFuture<CredentialLease> {
        self.0.issue(*self.0.recovered_generation.lock().unwrap())
    }
    fn finish_rejected(&self, _: &CredentialLease) -> ManagedFuture<()> {
        unused()
    }
    fn quiesce(&self) -> BoxFuture<'static, ()> {
        self.0.quiesced.store(true, Ordering::SeqCst);
        Box::pin(async {})
    }
}
struct FixturePlugin(Arc<Fixture>);
impl TypedPlugin for FixturePlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "managed-fixture"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as::<dyn ManagedProvider>(
                TypeKey::of::<dyn ManagedProvider>(),
                Arc::new(Provider(self.0.clone())),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for FixturePlugin {
    fn keys(&self) -> KeysOf<Self> {}
}
struct Capture(Arc<OnceLock<Arc<ManagedProviders>>>);
impl TypedPlugin for Capture {
    type Deps = Arc<ManagedProviders>;
    fn name(&self) -> &str {
        "capture-managed"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        owner: Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let _ = self.0.set(owner);
            Ok(Effect::Done)
        })
    }
}
impl Capability for Capture {
    fn keys(&self) -> KeysOf<Self> {}
}
async fn active(view: &rutis::FiberView) {
    let mut watch = view.watch();
    while watch.borrow().state != FiberState::Active {
        assert_ne!(
            watch.borrow().state,
            FiberState::Failed,
            "{:?}",
            *watch.borrow()
        );
        watch.changed().await.unwrap();
    }
}

async fn setup() -> (Ctx, Arc<Fixture>, Arc<ManagedProviders>, rutis::FiberView) {
    let root = Ctx::root().unwrap();
    let fixture = Arc::new(Fixture::default());
    *fixture.recovered_generation.lock().unwrap() = 1;
    let captured = Arc::new(OnceLock::new());
    let owner_plugin = root.plugin(capability_plugin(ManagedProvidersPlugin));
    let fixture_plugin = root.plugin(capability_plugin(FixturePlugin(fixture.clone())));
    let binding = root.plugin(capability_plugin(ManagedProviderBinding));
    let capture = root.plugin(capability_plugin(Capture(captured.clone())));
    for view in [&owner_plugin, &fixture_plugin, &binding, &capture] {
        active(view).await;
    }
    (root, fixture, captured.get().unwrap().clone(), binding)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abandoned_credential_and_recovery_waiters_release_every_issued_lease() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for recovery in [false, true] {
            let (root, fixture, owner, _) = setup().await;
            fixture.resume.notify_one();
            let initial = owner
                .credential("fixture", "model", &CancellationToken::new())
                .await
                .unwrap();
            let waiter = tokio::spawn(async move {
                let cancel = CancellationToken::new();
                if recovery {
                    owner.recover(&initial, &cancel).await
                } else {
                    drop(initial);
                    owner.credential("fixture", "model", &cancel).await
                }
            });
            while fixture.entered.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
            waiter.abort();
            assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));
            // The provider still finishes the accepted work; its result drops.
            fixture.resume.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while fixture.leases.released.lock().unwrap().len() < 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            root.shutdown().await.unwrap();
            let mut released = fixture.leases.released.lock().unwrap().clone();
            released.sort();
            assert_eq!(
                released,
                *fixture.leases.issued.lock().unwrap(),
                "recovery={recovery}"
            );
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovery_for_another_account_is_refused_and_released() {
    let (root, fixture, owner, _) = setup().await;
    fixture.resume.notify_one();
    let initial = owner
        .credential("fixture", "model", &CancellationToken::new())
        .await
        .unwrap();
    *fixture.recovered_generation.lock().unwrap() = 2;
    fixture.resume.notify_one();
    let changed = owner
        .recover(&initial, &CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert_eq!(changed.code, "account_changed");
    assert_eq!(*fixture.leases.released.lock().unwrap(), ["lease-1"]);
    drop(initial);
    root.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unbinding_withdraws_the_provider_after_quiescing_it() {
    let (root, fixture, owner, binding) = setup().await;
    let cancel = CancellationToken::new();
    assert_eq!(owner.providers(&cancel).await.unwrap().len(), 1);
    binding.shutdown().await.unwrap();
    assert!(fixture.quiesced.load(Ordering::SeqCst));
    assert!(owner.providers(&cancel).await.unwrap().is_empty());
    let unavailable = owner
        .credential("fixture", "model", &cancel)
        .await
        .err()
        .unwrap();
    assert_eq!(unavailable.code, "account_provider_unavailable");
    root.shutdown().await.unwrap();
}
