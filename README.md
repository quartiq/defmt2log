# `defmt2log`

Keep writing real `defmt` in code that also runs on the host.

`defmt2log` is a `defmt::Logger`/`#[global_logger]` that decodes
`defmt` frames and emits ordinary [`log`](https://docs.rs/log) records,
so you keep:

- full `defmt` format hints and syntax in the code
- `DEFMT_LOG` compile-time filtering
- normal host `log` tooling such as `RUST_LOG`, `env_logger`, and downstream
  `log` sinks

On Linux/ELF it can initialize without a special linker script in many cases:

- `init_from_current_exe()` for the normal host-binary case
- `init_from_merged_elf_path(path)` for any ELF path with a merged `.defmt`
  section
- `init_from_merged_elf_bytes(bytes)` for pre-merged ELF bytes

All initialization functions panic on failure.

## Usage

```rust
env_logger::init();
defmt2log::init_from_current_exe();
defmt::info!("word {=u32:#010x}", 0x1234u32);
```

- normal debug and release host binaries work as well as libtest unit
  tests, examples, and integration tests; they need one `defmt2log::init_from_*()`
- rustdoc doctest executables are worse: the bundled doctest rlib still
  contains `.defmt.info.*`, but the final `rust_out` test executable has
  them stripped.

Recommended default:

- build with `DEFMT_LOG=info`
- run with `RUST_LOG=info` (when using `env_logger`)
- leave `log` compile-time max-level features alone
- source locations are loaded on a best-effort basis; if they cannot be loaded,
  decoding still works and `defmt2log` warns once
- if `DEFMT_LOG` is unset or more restrictive than your callsites, normal
  `defmt::{trace,debug,info,warn,error}!` output is compiled out entirely; in
  that case `defmt2log` may still initialize successfully, but there is
  nothing for it to decode
- `DEFMT_LOG=off` and no `defmt::println!()` removes the need for a `defmt` `#[global_logger]`

## Filters

- `DEFMT_LOG` is the compile-time filter for `defmt`
- `RUST_LOG` is the runtime filter for the host `env_logger` `log` sink
- `max_level_*` and `release_max_level_*` affect ordinary host `log` callsites,
  not `defmt`

The important consequence is simple: `RUST_LOG` cannot bring back `defmt`
callsites that `DEFMT_LOG` compiled out.

`defmt::println!()` is decoded and printed directly to stdout, bypassing
`RUST_LOG`, with best-effort source location metadata.

## Avoid

- `DEFMT_LOG=trace` with `RUST_LOG=warn` unless you intentionally want to pay
  decode cost for logs the sink will hide
- using `max_level_*` features to control `defmt`
- using `init_from_merged_elf_bytes()` for a normal host executable; that API
  is only for ELFs that already contain a merged `.defmt` section.
- expecting `init_from_merged_elf_path(path)` to synthesize a table for an
  arbitrary non-running host binary without a merged `.defmt` section

## Limitations

- doctests tend to not produce output:
  the final doctest executable loses the split `.defmt.*` metadata even when
  the doctest itself still compiles and passes
- `defmt2log` is typically less efficient than pure `log`; the overhead is
  the sum of the defmt overheads: encoding, decoding, and formatting
- every compile-time-enabled `defmt` frame is decoded in-process
- `init_from_current_exe()` is Linux-oriented today:
  the split-`.defmt.*` current-executable path depends on loader-reported
  mappings via `findshlibs`
- `init_from_merged_elf_path()` and `init_from_merged_elf_bytes()` are the
  more portable modes: they work when the input already has a merged `.defmt`
  section
- native macOS current-executable support is not a supported path today
- host bitflags names require linker support that preserves `.defmt.end*`
  metadata; without that, host decoders only see the bitflags format tag and
  fall back to the raw numeric value
- a future host-side linker setup may preserve `.defmt.end*` and merge
  `.defmt.*` into one real `.defmt` section so `defmt-decoder::Table::parse()`
  can be used directly, but that is not a supported recipe yet
- With the synthesized table, the naive unmerged `DefmtSymbol::runtime_index: u16`
  can collide

## Alternatives

- only `log::*`: simplest host setup, but you give up efficient `defmt` format
  hints and `DEFMT_LOG` compile-time filtering.
- `defmt-or-log` or the `fmt.rs` trick: useful when one shared codebase must
  compile against either backend, but the shared callsites have to stay within
  the portable subset.
- `defmt-logger`: also aims at `defmt` -> `log` but it is old. `defmt2log` is built
  around current `defmt-decoder` `log` interop and preserves the normal host
  `log` pipeline end to end: `RUST_LOG` filtering, existing sinks, and other
  downstream `log` machinery.
- external decoding such as `defmt-print`: keeps full `defmt`, but moves the
  decode and process orchestration outside the program. `defmt2log` keeps the
  same logging stream inside the host process, merges with other `log` sources,
  and feeds ordinary `log` sinks directly.
