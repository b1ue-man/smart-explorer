//! The requests/authorized-devices read models live in
//! `crate::share::lifecycle_view`; re-exported here under their previous names.
pub(super) use crate::share::lifecycle_view::{
    authorized_device_views, request_views, AuthorizedDeviceView, RequestView,
};
