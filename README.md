# frep

frep is a fast find-and-replace tool. To replace the string "before" with the string "after" in the current directory:

```sh
frep before after
```

There are a number of command-line flags to change the behaviour of frep, such as:

- `--fixed-strings` (`-f`) to search without regex
- `--advanced-regex` (`-a`) to use advanced regex features such as negative lookahead (not enabled by default for improved performance)
- `--include-files` (`-I`) and `--exclude-files` (`-E`) to include or exclude files and directories using glob matching. For instance, `-I "*.rs, *.py"` matches all files with the `.rs` or `.py` extensions, and `-E "env/**"` excludes all files in the `env` directory

Run `frep --help` to see the full list of flags.

## Performance

frep is fast. Compared against other tools that also respect ignore files such as `.gitignore`, it is the fastest in many scenarios. Here is a benchmark for comparison, performing a find and replace across the entire [Linux kernel repo](https://github.com/torvalds/linux), finding and replacing the string "before" with "after":

<!-- BENCHMARK START -->
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `frep` | 3.772 ± 0.080 | 3.671 | 3.890 | 1.00 |
| `ripgrep + sd` | 3.951 ± 0.196 | 3.732 | 4.206 | 1.05 ± 0.06 |
| `fd + sd` | 10.028 ± 0.002 | 10.025 | 10.029 | 2.66 ± 0.06 |

<!-- BENCHMARK END -->

## Installation

<!-- TODO:
[![Packaging status](https://repology.org/badge/vertical-allrepos/frep.svg)](https://repology.org/project/frep/versions)
-->

### Prebuilt binaries

Download the appropriate binary for your system from the [releases page](https://github.com/thomasschafer/frep/releases/latest):

| Platform | Architecture | Download file |
|-|-|-|
| Linux | Intel/AMD | `*-x86_64-unknown-linux-musl.tar.gz` |
| Linux | ARM64 | `*-aarch64-unknown-linux-musl.tar.gz` |
| macOS | Apple Silicon| `*-aarch64-apple-darwin.tar.gz` |
| macOS | Intel | `*-x86_64-apple-darwin.tar.gz` |
| Windows | x64 | `*-x86_64-pc-windows-msvc.zip` |

After downloading, extract the binary and move it to a directory in your `PATH`.

### Cargo

Ensure you have cargo installed (see [here](https://doc.rust-lang.org/cargo/getting-started/installation.html)), then run:

```sh
cargo install frep
```

### Building from source

Ensure you have cargo installed (see [here](https://doc.rust-lang.org/cargo/getting-started/installation.html)), then run the following commands:

```sh
git clone git@github.com:thomasschafer/frep.git
cd frep
cargo install --path frep --locked
```
