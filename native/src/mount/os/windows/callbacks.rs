use super::{
    callbacks_io, callbacks_metadata, callbacks_mutation, callbacks_open, callbacks_unsupported,
    DokanOperations,
};

pub(super) fn operations() -> DokanOperations {
    DokanOperations {
        create_file: Some(callbacks_open::create_file),
        cleanup: Some(callbacks_mutation::cleanup),
        close_file: Some(callbacks_mutation::close_file),
        read_file: Some(callbacks_io::read_file),
        write_file: Some(callbacks_io::write_file),
        flush_file_buffers: Some(callbacks_io::flush_file_buffers),
        get_file_information: Some(callbacks_metadata::get_file_information),
        find_files: Some(callbacks_metadata::find_files),
        find_files_with_pattern: Some(callbacks_metadata::find_files_with_pattern),
        set_file_attributes: Some(callbacks_unsupported::set_file_attributes),
        set_file_time: Some(callbacks_unsupported::set_file_time),
        delete_file: Some(callbacks_mutation::delete_file),
        delete_directory: Some(callbacks_mutation::delete_directory),
        move_file: Some(callbacks_mutation::move_file),
        set_end_of_file: Some(callbacks_io::set_end_of_file),
        set_allocation_size: Some(callbacks_io::set_allocation_size),
        // Without DOKAN_OPTION_FILELOCK_USER_MODE, Dokany correctly owns file
        // lock enforcement in the driver and never calls these fields.
        lock_file: None,
        unlock_file: None,
        get_disk_free_space: Some(callbacks_metadata::get_disk_free_space),
        get_volume_information: Some(callbacks_metadata::get_volume_information),
        mounted: Some(callbacks_metadata::mounted),
        unmounted: Some(callbacks_metadata::unmounted),
        get_file_security: Some(callbacks_unsupported::get_file_security),
        set_file_security: Some(callbacks_unsupported::set_file_security),
        find_streams: Some(callbacks_unsupported::find_streams),
    }
}
