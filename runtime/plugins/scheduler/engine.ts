import { randomUUID } from 'node:crypto'
import type { Sessions } from '../../../npm/plugin.js'
import { effective, ScheduleError, within, type Job } from './data.ts'
import { RunHistory } from './history.ts'
import type { Jobs } from './jobs.ts'
import { unloadingAfterCall } from './requests.ts'
import { terminal, type Run, type RunStore } from './store.ts'
import { latestWindow, windowAfter } from './windows.ts'

const pause = (milliseconds: number) => new Promise<void>(resolve => setTimeout(resolve, milliseconds))
const escaped = (text: string) => text.replace(/[&<>"']/g, character => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' })[character]!)

export class Scheduler {
  readonly jobs: Jobs
  readonly sessions: Sessions
  readonly store: RunStore
  closing = false
  fault: unknown
  readonly work = new Map<string, Promise<unknown>>()
  private pruning = false
  private lastPrune = -Infinity

  constructor(jobs: Jobs, sessions: Sessions, store: RunStore) {
    this.jobs = jobs; this.sessions = sessions; this.store = store
  }

  async track<T>(operation: () => Promise<T>, key: string = randomUUID()): Promise<T> {
    if (this.closing) throw new ScheduleError('service_unavailable', unloadingAfterCall)
    if (this.fault) throw this.fault
    const work = operation()
    this.work.set(key, work)
    try { return await work }
    finally { this.work.delete(key) }
  }

  async restore() {
    const runs = await this.store.read()
    for (const run of runs.filter(run => !terminal(run))) {
      this.background(run.run_id, () => this.resume(run))
    }
    await this.prune()
  }

  startTimer() {
    this.background('timer', async () => {
      while (!this.closing && !this.fault) {
        try { await this.tick() }
        catch (error) { console.error('Scheduler timer', error instanceof Error ? error.message : String(error)) }
        await pause(1000)
      }
    })
  }

  changed() {
    if (!this.closing && !this.fault && !this.work.has('wake')) this.background('wake', () => this.tick())
  }

  private background(key: string, operation: () => Promise<unknown>) {
    void this.track(operation, key).catch(error => {
      this.fault = error
      console.error('Scheduler', error instanceof Error ? error.message : String(error))
    }).finally(() => {
      if (!['timer', 'retention', 'wake'].includes(key) && !this.closing && !this.fault) this.requestPrune()
    })
  }

  private requestPrune() {
    if (!this.pruning) this.background('retention', () => this.prune())
  }

  async close() {
    this.closing = true
    while (this.work.size) await Promise.allSettled([...this.work.values()])
    await this.store.scope.close()
  }

  async tick() {
    const { document } = await this.jobs.read()
    const runs = await this.store.read()
    for (const job of document.jobs.filter(job => job.enabled)) {
      if (this.closing || this.fault) break
      const schedule = job.schedule
      if ('at' in schedule && runs.some(run => run.job_id === job.id && run.window === new Date(schedule.at).toISOString()
        && (terminal(run) || run.input_event_id))) {
        await this.jobs.settleWindow(job, undefined, true)
        continue
      }
      const windows = runs.filter(run => run.job_id === job.id && run.window !== 'manual').map(run => run.window)
      const previous = windows.length ? Date.parse(windows.sort().at(-1)!) : Date.parse(job.enabled_at!) - 1
      const next = windowAfter(job, previous)
      if (next === undefined || next > Date.now()) continue
      if (Date.now() - next >= 60_000) {
        await this.accept(job.id, new Date(latestWindow(job, Date.now())).toISOString(), true)
      } else await this.accept(job.id, new Date(next).toISOString())
    }
    if (Date.now() - this.lastPrune >= 60_000) this.requestPrune()
  }

  async accept(jobId: string, window = 'manual', missed = false): Promise<Run> {
    const { document } = await this.jobs.read()
    const snapshot = document.jobs.find(job => job.id === jobId)
    if (!snapshot) throw new ScheduleError('schedule_not_found')
    const accepted = await this.store.update(runs => {
      const existing = window === 'manual' ? undefined : runs.find(run => run.job_id === jobId && run.window === window)
      if (existing) return { run: existing, fresh: false }
      const now = new Date(Date.now()).toISOString()
      const reason = missed ? 'missed' : !snapshot.enabled ? 'disabled'
        : runs.some(run => run.job_id === jobId && !terminal(run)) ? 'overlap'
        : runs.filter(run => !terminal(run)).length >= document.max_concurrent_runs ? 'capacity' : undefined
      const deadline = new Date(Date.now() + snapshot.max_run_minutes * 60_000)
      if (!reason && (!Number.isFinite(deadline.getTime()) || deadline.getUTCFullYear() > 9999)) {
        throw new ScheduleError('schedule_invalid', 'Run deadline exceeds the persisted timestamp range')
      }
      const runId = randomUUID()
      const run: Run = { run_id: runId, job_id: jobId, window, state: reason ? missed ? 'missed' : 'skipped' : 'launching',
        created_at: now, submission_id: `scheduler:${runId}`, detail: reason ?? '',
        ...(reason ? { ended_at: now } : {
          deadline_at: deadline.toISOString(),
          ...(snapshot.created_by === 'user' ? {} : { report_to: snapshot.created_by.tool }),
        }),
      }
      runs.push(run)
      return { run, fresh: true }
    })
    if (accepted.fresh && !terminal(accepted.run)) this.background(accepted.run.run_id, () => this.execute(snapshot, accepted.run))
    else if (accepted.fresh) {
      if (window !== 'manual' && 'at' in snapshot.schedule) await this.jobs.settleWindow(snapshot, undefined, true)
      this.requestPrune()
    }
    return accepted.run
  }

  private async check(snapshot: Job, runId: string): Promise<Run | undefined> {
    const { document } = await this.jobs.read()
    const run = (await this.store.read()).find(run => run.run_id === runId)!
    if (terminal(run) || this.closing || this.fault) return undefined
    if (Date.now() >= Date.parse(run.deadline_at!)) {
      await this.finish(runId, 'timed_out', 'deadline_before_input')
      return undefined
    }
    const current = document.jobs.find(job => job.id === snapshot.id)
    if (!current || current.revision !== snapshot.revision || !current.enabled) {
      await this.finish(runId, 'skipped', current && current.revision === snapshot.revision ? 'disabled' : 'changed')
      return undefined
    }
    return run
  }

  private async execute(snapshot: Job, accepted: Run) {
    const submitted = await this.submit(snapshot, accepted)
    try { await this.jobs.settleWindow(snapshot, submitted?.sessionId, accepted.window !== 'manual' && 'at' in snapshot.schedule) }
    catch (error) { console.error('Scheduler job settlement', error instanceof Error ? error.message : String(error)) }
    if (submitted) await this.follow({ ...accepted, session_id: submitted.sessionId, input_event_id: submitted.receipt.inputId }, submitted.receipt.sequence)
  }

  private async submit(snapshot: Job, accepted: Run) {
    if (!await this.check(snapshot, accepted.run_id)) return
    let sessionId: string
    if (snapshot.mode === 'create' || !snapshot.session_id) {
      let reply
      try {
        reply = await this.sessions.launch({ cwd: snapshot.cwd, policy: effective(snapshot),
          ...(snapshot.agent_id ? { agent: snapshot.agent_id } : {}), ...(snapshot.model ? { model: snapshot.model } : {}),
          ...(snapshot.created_by === 'user' ? {} : { starter: {
            sessionId: snapshot.created_by.tool.session_id, branchId: snapshot.created_by.tool.branch_id,
          } }),
        })
      } catch {
        await this.finish(accepted.run_id, 'lost', 'launch_reply_unknown')
        return
      }
      if (!reply.ok) {
        await this.finish(accepted.run_id, 'failed', reply.error.code)
        return
      }
      sessionId = reply.value.sessionId
      await this.store.update(runs => Object.assign(runs.find(run => run.run_id === accepted.run_id)!, {
        session_id: sessionId, session_origin: 'launch', policy: reply.value.policy,
      }))
    } else {
      let live, actual
      try {
        live = snapshot.created_by === 'user' ? await this.jobs.permissions.permissionDefaults({})
          : (await this.jobs.permissions.permission({ sessionId: snapshot.created_by.tool.session_id, branchId: snapshot.created_by.tool.branch_id })).policy
        actual = (await this.jobs.permissions.permission({ sessionId: snapshot.session_id, branchId: 'main' })).policy
      } catch {
        if (await this.check(snapshot, accepted.run_id)) await this.finish(accepted.run_id, 'skipped', 'policy_read_failed')
        return
      }
      if (!within(actual, effective(snapshot)) || !within(actual, live)) {
        if (await this.check(snapshot, accepted.run_id)) await this.finish(accepted.run_id, 'skipped', 'policy')
        return
      }
      sessionId = snapshot.session_id
      await this.store.update(runs => Object.assign(runs.find(run => run.run_id === accepted.run_id)!, {
        session_id: sessionId, session_origin: 'reuse', policy: actual,
      }))
    }
    if (!await this.check(snapshot, accepted.run_id)) return
    let receipt, inputReply
    try {
      inputReply = await this.sessions.input({ sessionId, branchId: 'main', mode: 'queue', submissionId: accepted.submission_id,
        rawBlocks: [{ text: `<dim-scheduler kind="scheduled-run" job="${escaped(snapshot.id)}" run="${escaped(accepted.run_id)}">${escaped(snapshot.name)}</dim-scheduler>` }],
        content: [{ type: 'text', text: snapshot.prompt }],
      })
    } catch { /* An admitted input settles before its uncertain reply is reconciled. */ }
    if (inputReply?.ok) receipt = inputReply.value
    else if (inputReply && !['reloading', 'publication_unknown', 'ambiguous'].includes(inputReply.error.code)) {
      await this.finish(accepted.run_id, 'failed', inputReply.error.code)
      return
    }
    if (!receipt) {
      const current = (await this.store.read()).find(run => run.run_id === accepted.run_id)!
      receipt = await this.lookup(current)
      if (!receipt) return
    }
    await this.store.update(runs => Object.assign(runs.find(run => run.run_id === accepted.run_id)!, {
      state: 'submitted', input_event_id: receipt.inputId,
    }))
    return { sessionId, receipt }
  }

  private async lookup(run: Run) {
    while (!this.closing) {
      let reply
      try { reply = await this.sessions.lookup({ sessionId: run.session_id!, branchId: 'main', submissionId: run.submission_id }) }
      catch { await this.finish(run.run_id, 'lost', 'input_lookup_failed'); return }
      if (reply.ok) {
        if (reply.value.status === 'accepted') return reply.value
        await this.finish(run.run_id, 'lost', 'input_unknown')
        return
      }
      if (reply.error.code !== 'reloading') { await this.finish(run.run_id, 'lost', reply.error.code); return }
      await pause(1000)
    }
  }

  private async resume(run: Run) {
    if (!run.session_id) { await this.finish(run.run_id, 'lost', 'launch_identity_unknown'); return }
    if (!run.input_event_id) {
      const found = await this.lookup(run)
      if (!found) return
      await this.store.update(runs => Object.assign(runs.find(value => value.run_id === run.run_id)!, { state: 'submitted', input_event_id: found.inputId }))
      await this.follow({ ...run, input_event_id: found.inputId }, found.sequence)
    } else await this.follow(run)
  }

  private async follow(run: Run, after = 0) {
    const history = new RunHistory(this.sessions, run.session_id!, run.input_event_id!, after)
    let stopIssued = false
    while (!this.closing && !this.fault) {
      if (!await history.read()) { await pause(1000); continue }
      if (history.outcome) {
        await this.finish(run.run_id, stopIssued && history.outcome.state === 'cancelled' ? 'timed_out' : history.outcome.state, history.outcome.detail)
        return
      }
      if (Date.now() >= Date.parse(run.deadline_at!)) {
        if (history.applied && history.interrupted && !history.consumed) {
          await this.finish(run.run_id, 'timed_out', 'interrupted_before_consumption')
          return
        }
        if (!history.consumed) {
          let withdrawn
          try { withdrawn = await this.sessions.withdraw({ sessionId: run.session_id!, branchId: 'main', inputId: run.input_event_id!, reason: 'scheduled run timed out' }) }
          catch { /* Re-read the actual prefix after an uncertain withdrawal. */ }
          if (withdrawn?.ok && withdrawn.value.status === 'deleted') { await this.finish(run.run_id, 'timed_out', 'withdrawn_at_deadline'); return }
          // notHeld is not consumption; read a complete prefix again before deciding.
          await history.read()
        } else if (!stopIssued) {
          stopIssued = true
          // shortcut: branch-addressed stop may reach a later turn; revisit with an agreed targeting contract.
          try { await this.sessions.stop({ sessionId: run.session_id!, branchId: 'main', reason: 'scheduled run timed out' }) }
          catch { /* The following history read decides what actually ended. */ }
        }
      } else if (history.consumed && run.state !== 'running') {
        await this.store.update(runs => { runs.find(value => value.run_id === run.run_id)!.state = 'running' })
        run.state = 'running'
      }
      const remaining = Date.parse(run.deadline_at!) - Date.now()
      await pause(remaining > 0 ? Math.min(1000, remaining) : 1000)
    }
  }

  private async finish(runId: string, state: Run['state'], detail: string) {
    const run = await this.store.update(runs => {
      const run = runs.find(run => run.run_id === runId)!
      if (terminal(run)) return undefined
      run.state = state
      run.detail = detail
      run.ended_at = new Date(Date.now()).toISOString()
      return run
    })
    if (!run?.report_to || !['completed', 'failed', 'cancelled', 'timed_out', 'lost'].includes(run.state)) return
    let failure: string | undefined
    try {
      const reply = await this.sessions.input({ sessionId: run.report_to.session_id, branchId: run.report_to.branch_id,
        mode: 'steer', submissionId: `scheduler-report:${run.run_id}`, rawBlocks: [{ text:
          `Scheduled job ${run.job_id}, run ${run.run_id}: ${run.state} (${run.detail}).${run.session_id ? ` Session ${run.session_id}.` : ''} Inspect schedule.history for this job.`,
        }], content: [],
      })
      if (!reply.ok) failure = reply.error.code
    } catch { failure = 'bridge_error' }
    if (failure) await this.store.update(runs => {
      runs.find(value => value.run_id === runId)!.detail += `; report_failed: ${failure}`
    })
  }

  async prune() {
    if (this.pruning) return
    this.pruning = true
    this.lastPrune = Date.now()
    try {
      const runs = await this.store.read()
      // A run's job is committed first; read its references after the run snapshot.
      const { document } = await this.jobs.read()
      const keep = new Set(runs.filter(run => !terminal(run) || this.work.has(run.run_id)).map(run => run.run_id))
      for (const job of document.jobs) {
        const newest = runs.filter(run => run.job_id === job.id && terminal(run))
          .sort((a, b) => b.ended_at!.localeCompare(a.ended_at!) || b.run_id.localeCompare(a.run_id)).slice(0, 100)
        for (const run of newest) keep.add(run.run_id)
      }
      const references = new Set([...document.jobs.filter(job => job.mode === 'reuse').map(job => job.session_id),
        ...runs.filter(run => keep.has(run.run_id)).map(run => run.session_id)].filter((id): id is string => id !== undefined))
      for (const sessionId of references) {
        const ownership = runs.filter(run => run.session_id === sessionId && run.session_origin === 'launch')
          .sort((a, b) => b.created_at.localeCompare(a.created_at) || b.run_id.localeCompare(a.run_id))[0]
        if (ownership) keep.add(ownership.run_id)
      }
      const expired = runs.filter(run => !keep.has(run.run_id))
      const deletions = new Set(expired.filter(run => run.session_origin === 'launch' && !references.has(run.session_id!)).map(run => run.session_id!))
      for (const sessionId of deletions) {
        try {
          const reply = await this.sessions.delete({ sessionId })
          if (!reply.ok) throw new ScheduleError(reply.error.code)
        } catch (error) {
          for (const run of expired.filter(run => run.session_id === sessionId && run.session_origin === 'launch')) keep.add(run.run_id)
          console.error('Scheduler cleanup', error instanceof Error ? error.message : String(error))
        }
      }
      const remove = new Set(expired.filter(run => !keep.has(run.run_id)).map(run => run.run_id))
      if (remove.size) await this.store.update(current => {
        for (let index = current.length - 1; index >= 0; index--) if (remove.has(current[index].run_id) && !this.work.has(current[index].run_id)) current.splice(index, 1)
      })
    } catch (error) {
      // An unreadable Settings document or record file never means its references vanished.
      console.error('Scheduler retention', error instanceof Error ? error.message : String(error))
    } finally { this.pruning = false }
  }
}
