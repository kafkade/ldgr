//! Benchmark Argon2id key derivation with different parameter sets.
//!
//! Run with:
//!   cargo run --example bench_argon2 -p ldgr-core --release
//!
//! This measures wall-clock time for `derive_master_key` with each
//! preset to help choose the right security/speed tradeoff per platform.

use std::time::Instant;

use ldgr_core::crypto::{Argon2Params, derive_master_key};

fn main() {
    let password = b"correct horse battery staple";
    let salt = b"ldgr-benchmark-salt-0123456";

    let configs: Vec<(&str, Argon2Params)> = vec![
        (
            "Desktop (256 MB, 3 iter, 4 threads)",
            Argon2Params::desktop(),
        ),
        (
            "Mobile  ( 64 MB, 4 iter, 2 threads)",
            Argon2Params::mobile(),
        ),
        ("WASM    ( 64 MB, 3 iter, 1 thread) ", Argon2Params::wasm()),
        (
            "Desktop Standard (128 MB, 3 iter, 4 threads)",
            Argon2Params {
                memory_cost_kib: 128 * 1024,
                iterations: 3,
                parallelism: 4,
            },
        ),
        (
            "Mobile Low (32 MB, 6 iter, 2 threads)",
            Argon2Params {
                memory_cost_kib: 32 * 1024,
                iterations: 6,
                parallelism: 2,
            },
        ),
    ];

    println!("Argon2id Key Derivation Benchmark");
    println!("=================================");
    println!();
    println!("{:<48} {:>10} {:>10}", "Configuration", "Time", "Target");
    println!("{}", "-".repeat(72));

    for (name, params) in &configs {
        let target = match *name {
            n if n.contains("Desktop") => "< 1.5s",
            n if n.contains("Mobile") => "< 2.0s",
            n if n.contains("WASM") => "< 3.0s",
            _ => "—",
        };

        // Warm-up run (excluded from measurement)
        let _ = derive_master_key(password, salt, params);

        // Timed run (average of 3)
        let mut total = std::time::Duration::ZERO;
        let runs = 3;
        for _ in 0..runs {
            let start = Instant::now();
            let _ = derive_master_key(password, salt, params);
            total += start.elapsed();
        }
        let avg = total / runs;

        let ms = avg.as_millis();
        let time_str = if ms >= 1000 {
            format!("{:.2}s", avg.as_secs_f64())
        } else {
            format!("{ms}ms")
        };

        println!("{name:<48} {time_str:>10} {target:>10}");
    }

    println!();
    println!("Note: Run with --release for accurate measurements.");
    println!("      WASM timing must be measured in a browser environment.");
}
