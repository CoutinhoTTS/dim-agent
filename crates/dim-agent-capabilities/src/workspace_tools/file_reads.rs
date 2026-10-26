//! Branch-level record of the text files this branch has read.
//!
//! A binding is what the model has seen of a file on this branch: its raw
//! byte revision and the text format `read` resolved (ADR agent-019).
//! Overwriting an existing file requires a binding from this branch.

use dim_agent_runtime::Names;
use rutis::host::{CtxHost, Key};
use rutis::{Ctx, Plugin};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    /// Only established by an explicit `read.encoding`; never guessed.
    Legacy {
        /// The requested name, lowercased, as echoed to the model.
        name: String,
        encoding: &'static encoding_rs::Encoding,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingSource {
    Bom,
    User,
    Utf8Policy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bom {
    None,
    Utf8,
    Utf16Le,
    Utf16Be,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eol {
    None,
    Lf,
    Crlf,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFileBinding {
    /// SHA-256 of the raw file bytes.
    pub raw_revision: [u8; 32],
    pub encoding: TextEncoding,
    pub encoding_source: EncodingSource,
    pub bom: Bom,
    pub eol: Eol,
}

/// Bindings keyed by canonical path; lives as long as the branch instance.
#[derive(Default)]
pub struct FileReads {
    bindings: Mutex<HashMap<PathBuf, TextFileBinding>>,
}
impl FileReads {
    pub(crate) fn get(&self, path: &Path) -> Option<TextFileBinding> {
        self.bindings.lock().unwrap().get(path).cloned()
    }
    pub(crate) fn set(&self, path: PathBuf, binding: TextFileBinding) {
        self.bindings.lock().unwrap().insert(path, binding);
    }
}

pub struct FileReadsPlugin {
    reads: Key<FileReads>,
}
impl FileReadsPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            reads: names.key()?,
        })
    }
}
impl Plugin for FileReadsPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "FileReads"
    }
    async fn apply(&self, ctx: &Ctx, (): Self::Deps) -> rutis::Result {
        ctx.provide_at(self.reads.clone(), Arc::new(FileReads::default()))?;
        Ok(())
    }
}
