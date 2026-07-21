/// Static guarantees needed before exposing a backend as a writable mounted
/// filesystem. They describe safe commit primitives, not whether credentials
/// currently permit a particular operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StagedWriteCapabilities {
    pub create: bool,
    pub replace: bool,
    pub namespace_replace: bool,
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
