/// Static guarantees needed before exposing a backend as a writable mounted
/// filesystem. They describe safe commit primitives, not whether credentials
/// currently permit a particular operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StagedWriteCapabilities {
    pub create: bool,
    pub replace: bool,
    pub namespace_replace: bool,
}

/// One coherent capability snapshot for exposing an exact backend path as a
/// mounted filesystem. Backends whose active implementation can change while
/// resolving a path (notably a Share peer) override `Backend`'s combined probe
/// so both guarantees describe the same resolved target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MountPathCapabilities {
    pub staged_write: StagedWriteCapabilities,
    pub root_confinement: RootConfinement,
}

impl StagedWriteCapabilities {
    pub const fn complete() -> Self {
        Self {
            create: true,
            replace: true,
            namespace_replace: true,
        }
    }

    pub const fn supports_mounted_writes(self) -> bool {
        self.create && self.replace && self.namespace_replace
    }

    pub fn intersect(&mut self, other: Self) {
        self.create &= other.create;
        self.replace &= other.replace;
        self.namespace_replace &= other.namespace_replace;
    }
}

/// Whether a backend itself confines every filesystem operation to the exact
/// root supplied by the caller. `Enforced` is reserved for a kernel sandbox or
/// a provider namespace whose object lookup cannot traverse outside that root.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RootConfinement {
    #[default]
    Unverified,
    Enforced,
}

impl RootConfinement {
    pub const fn is_enforced(self) -> bool {
        matches!(self, Self::Enforced)
    }
}

pub(super) fn staged_write_defaults<B: super::Backend + ?Sized>(
    backend: &B,
) -> StagedWriteCapabilities {
    StagedWriteCapabilities {
        create: false,
        replace: backend.rename_overwrites(),
        namespace_replace: backend.rename_overwrites(),
    }
}

pub(super) fn mount_path_defaults<B: super::Backend + ?Sized>(
    backend: &B,
    root: &str,
) -> super::VfsResult<MountPathCapabilities> {
    Ok(MountPathCapabilities {
        staged_write: backend.staged_write_capabilities(root),
        root_confinement: backend.root_confinement(root),
    })
}
