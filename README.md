# `defmt2log`

Use `defmt` callsites in host code and forward them to the normal
[`log`](https://docs.rs/log) pipeline.

This lets shared code keep `defmt` formatting and `DEFMT_LOG` compile-time
filtering while host tests, examples, and binaries still use `RUST_LOG`,
`env_logger`, and other `log` sinks.

## Usage

```rust
env_logger::init();
defmt2log::init();

defmt::info!("word {=u32:#010x}", 0x1234u32);
```

`init()` loads defmt metadata from the current executable. Calling it is
optional: the first defmt frame initializes lazily too. Call it explicitly when
initialization errors should happen before the first log.

Unlike an embedded binary linked with `defmt.x`, a host binary has no linker
provided default timestamp. The application must define one; use
`defmt::timestamp!("")` if no timestamp should be encoded.

Typical setup:

- build with `DEFMT_LOG=info`
- run with `RUST_LOG=info`
- leave `log` compile-time max-level features alone

## Filtering

`DEFMT_LOG` is the compile-time filter for `defmt` callsites. If it compiles a
callsite out, `RUST_LOG` cannot bring it back.

`RUST_LOG` is the runtime filter for the host `log` sink. Setting `RUST_LOG`
more restrictively than `DEFMT_LOG` still pays the defmt encode/decode cost for
frames the sink will ignore.

`defmt::println!()` is decoded and printed directly to stdout, bypassing
`RUST_LOG`.

## Limitations

- `init()` reads defmt metadata from the running executable. It does not decode
  frames from arbitrary non-running binaries.
- Dynamically loaded objects are not decoded today. Their frames may reach the
  process-global defmt logger, but `defmt2log` only loads metadata from the main
  executable.
- Source locations are best-effort. If they cannot be loaded, decoding still
  works and `defmt2log` logs a warning.
- Rustdoc doctest executables tend to lose the split `.defmt.*` metadata.
- Native macOS current-executable support is not tested today.
