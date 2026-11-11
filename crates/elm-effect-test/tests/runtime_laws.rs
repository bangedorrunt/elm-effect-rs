//! The same carrier laws on both executors.

use elm_effect::Runtime;
use elm_effect_test::{DeterministicExecutor, runtime_laws};

#[test]
fn deterministic_executor_passes_the_carrier_laws() {
    let driver = DeterministicExecutor::new();
    let rt = Runtime::with_executor(driver.clone());
    let results = runtime_laws::run(&rt, &|| {
        let _ = driver.run_until_idle();
    });
    assert_laws(&results);
}

#[test]
fn tokio_executor_passes_the_carrier_laws() {
    let rt = Runtime::new();
    let results = runtime_laws::run(&rt, &|| {});
    assert_laws(&results);
}

fn assert_laws(results: &[runtime_laws::LawResult]) {
    let failures: Vec<&runtime_laws::LawResult> = results
        .iter()
        .filter(|result| result.outcome.is_err())
        .collect();
    assert!(failures.is_empty(), "failed carrier laws:\n{failures:#?}");
    println!("carrier laws: {} passed", results.len());
}
