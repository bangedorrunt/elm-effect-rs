//! Every public error type is a `std::error::Error`.
//!
//! A caller must be able to `?` these into `anyhow`, `Box<dyn Error>`, or a
//! host error enum, and get a `source()` chain. This test is the compile-time
//! pin: drop an impl and it stops building.

use elm_effect::{
    CauseError, ConfigError, Order, ProviderError, QueueError, RandomError, RemoteError,
};

fn is_error<T: std::error::Error>() {}

#[test]
fn public_error_types_implement_std_error() {
    is_error::<ConfigError>();
    is_error::<ProviderError>();
    is_error::<QueueError>();
    is_error::<RandomError>();
    is_error::<CauseError>();
    is_error::<RemoteError<std::io::Error>>();
    // `Order` has no error role; naming it keeps the import list honest if a
    // future error type is added beside it.
    let _ = std::any::type_name::<Order<u32>>();
}

#[test]
fn an_error_can_be_boxed_and_displayed() {
    let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(QueueError::Interrupted);
    assert_eq!(boxed.to_string(), "Interrupted");
    let boxed: Box<dyn std::error::Error> = Box::new(ProviderError::source("etcd down"));
    assert!(boxed.to_string().contains("etcd down"));
}
