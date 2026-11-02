//! Portable authorization and wire contracts of the optional login broker.
pub mod authority;
pub mod capture_wire;
pub mod gateway_types;
pub mod gateway_wire;
pub mod image_receive;
pub mod image_send;
pub mod machine;
pub mod manifest;
pub mod model;
pub mod viewer_guard;
pub mod wire;

#[cfg(test)]
mod batches_authority_task_tests;
#[cfg(test)]
mod batches_capture_task_tests;
#[cfg(test)]
mod batches_image_receive_task_tests;
