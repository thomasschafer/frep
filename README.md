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

frep is fast. Below is a benchmark for comparison, performing a find and replace across the entire [Linux kernel repo](https://github.com/torvalds/linux), finding and replacing the string "before" with "after":

<!-- BENCHMARK START -->
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `frep` | 4.789 ± 0.214 | 4.630 | 5.161 | 1.06 ± 0.05 |
| `ripgrep + sd` | 5.178 ± 0.083 | 5.075 | 5.274 | 1.14 ± 0.03 |
| `fastmod` | 4.539 ± 0.079 | 4.464 | 4.642 | 1.00 |
| `fd + sd` | 10.010 ± 0.000 | 10.010 | 10.011 | 2.21 ± 0.04 |

<!-- BENCHMARK END -->

## Installation

<!-- TODO:
[![Packaging status](https://repology.org/badge/vertical-allrepos/frep.svg)](https://repology.org/project/frep/versions)
-->

### Homebrew

On macOS and Linux, you can install frep using Homebrew:

```sh
brew install thomasschafer/tap/frep
```

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
