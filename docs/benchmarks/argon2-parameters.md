# Argon2id Parameter Benchmarks

Benchmarks for the Argon2id key derivation function across different parameter
configurations. These results inform the default parameters for each platform.

## How to Run

```sh
cargo run --example bench_argon2 -p ldgr-core --release
```

## Results

Measured on Windows x86_64, AMD Ryzen / Intel Core class CPU (representative desktop hardware).
All times are wall-clock averages of 3 runs in release mode.

| Configuration | Memory | Iterations | Parallelism | Time | Target | Status |
| --- | --- | --- | --- | --- | --- | --- |
| **Desktop** (default) | 256 MB | 3 | 4 | ~485 ms | < 1.5s | ✅ Pass |
| Desktop Standard | 128 MB | 3 | 4 | ~231 ms | < 1.5s | ✅ Pass |
| **Mobile** (default) | 64 MB | 4 | 2 | ~142 ms | < 2.0s | ✅ Pass |
| Mobile Low | 32 MB | 6 | 2 | ~98 ms | < 2.0s | ✅ Pass |
| **WASM** (default) | 64 MB | 3 | 1 | ~109 ms | < 3.0s | ✅ Pass* |

\* WASM time measured on native; actual browser/WASM timing will be slower
(typically 2–5× due to WASM overhead). The 109 ms native measurement suggests
~220–550 ms in WASM, well within the 3s target.

## Chosen Defaults

### Desktop: 256 MB, 3 iterations, 4 threads

The highest security configuration that stays under the 1.5s target. This
matches the OWASP recommendation for Argon2id (minimum 19 MiB, 2 iterations).
Our configuration significantly exceeds the minimum.

- 256 MB memory makes GPU/ASIC attacks expensive
- 4 threads leverages modern multi-core CPUs
- 3 iterations provide adequate time cost without excessive latency

### Mobile: 64 MB, 4 iterations, 2 threads

Optimized for iOS/Android devices with less memory and fewer cores:

- 64 MB is feasible on devices with 3+ GB RAM (standard since ~2018)
- Higher iteration count (4 vs 3) compensates for reduced memory cost
- 2 threads matches typical mobile CPU core availability during foreground use

### WASM: 64 MB, 3 iterations, 1 thread

Constrained by browser/WASM limitations:

- Single-threaded since WASM threading (SharedArrayBuffer) is not universally
  available and requires specific HTTP headers (COOP/COEP)
- 64 MB is manageable within typical browser tab memory budgets
- 3 iterations balance security with user-perceived latency in the browser

## Security Notes

- All configurations produce a 256-bit (32-byte) master key
- Parameters are stored in the vault header and can be upgraded on password change
- The `test()` preset (64 KiB, 1 iteration, 1 thread) is for unit tests only
  and must never be used in production

## Future Work

- Run WASM benchmarks in actual browser environments (Chrome, Firefox, Safari)
- Benchmark on specific iOS devices (iPhone 12+, iPad Air 4+)
- Consider adaptive parameter selection based on device capability detection
- Re-evaluate parameters annually as hardware improves
