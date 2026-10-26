//! Portable authorization and wire contracts of the optional login broker.
pub(crate) mod authority;
pub(crate) mod manifest;
pub(crate) mod model;
pub(crate) mod wire;
pub(crate) mod capture_wire;
pub(crate) mod machine;
pub(crate) mod gateway_types;
pub(crate) mod gateway_wire;
pub(crate) mod image_receive;
pub(crate) mod image_send;
pub(crate) mod viewer_guard;

#[cfg(test)]
mod batches_authority_task_tests;
#[cfg(test)]
mod batches_capture_task_tests;
#[cfg(test)]
mod batches_image_receive_task_tests;
