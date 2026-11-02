//! The storage crate's own test binary links no higher module, so it binds
//! the stand-ins of `test_ports` to the ports of `vfs` (D13).
use crate::test_ports::{NoLocalServerCopy, NoSealedSpools, NoUrlBackends, PlainEndpointRules};

crate::provide_port!(smart_explorer_port_vfs_sftp: dyn crate::vfs::UrlBackends = NoUrlBackends);
crate::provide_port!(smart_explorer_port_vfs_smb: dyn crate::vfs::UrlBackends = NoUrlBackends);
crate::provide_port!(smart_explorer_port_vfs_sealed_spools: dyn crate::vfs::SealedSpools = NoSealedSpools);
crate::provide_port!(
    smart_explorer_port_vfs_sync_endpoints: dyn crate::vfs::SyncEndpointRules = PlainEndpointRules
);
crate::provide_port!(
    smart_explorer_port_vfs_local_server_copy: dyn crate::vfs::LocalServerCopy = NoLocalServerCopy
);
