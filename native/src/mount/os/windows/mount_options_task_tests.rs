use super::{
    DokanOptions, OPTION_ALLOW_IPC_BATCHING, OPTION_CASE_SENSITIVE, OPTION_CURRENT_SESSION,
    OPTION_MOUNT_MANAGER, OPTION_WRITE_PROTECT,
};

#[test]
fn mount_batching_task_preserves_unrelated_configuration_on_every_attempt() {
    let mount_point = [b'Z' as u16, b':' as u16, b'\\' as u16, 0];
    let unc_name = [0u16];
    for case_sensitive in [0, OPTION_CASE_SENSITIVE] {
        for write_protect in [0, OPTION_WRITE_PROTECT] {
            for single_thread in [0, 1] {
                let retained = OPTION_CURRENT_SESSION | OPTION_MOUNT_MANAGER
                    | case_sensitive | write_protect | (1 << 30);
                let mut options = DokanOptions {
                    options: retained,
                    single_thread,
                    global_context: 0x1234,
                    mount_point: mount_point.as_ptr(),
                    unc_name: unc_name.as_ptr(),
                    timeout: 300_000,
                    allocation_unit_size: 4096,
                    sector_size: 4096,
                    volume_security_descriptor_length: 1,
                    ..DokanOptions::default()
                };
                options.volume_security_descriptor[0] = 42;
                let version = options.version;
                for _attempt in 0..3 {
                    // The DLL can set the caller's flag during worker setup.
                    options.options |= OPTION_ALLOW_IPC_BATCHING;
                    options.prepare_for_create();
                    assert_eq!(options.options, retained);
                    assert_eq!(options.single_thread, single_thread);
                    assert_eq!(options.version, version);
                    assert_eq!(options.global_context, 0x1234);
                    assert_eq!(options.mount_point, mount_point.as_ptr());
                    assert_eq!(options.unc_name, unc_name.as_ptr());
                    assert_eq!(options.timeout, 300_000);
                    assert_eq!(options.allocation_unit_size, 4096);
                    assert_eq!(options.sector_size, 4096);
                    assert_eq!(options.volume_security_descriptor_length, 1);
                    assert_eq!(options.volume_security_descriptor[0], 42);
                }
            }
        }
    }
}
