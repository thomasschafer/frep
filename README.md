# frep

frep is a fast find-and-replace tool. To replace the string "before" with the string "after" in the current directory:

```sh
frep before after
```

There are a number of command-line flags to change the behaviour of frep, such as `--fixed-strings` to search without regex. Run `frep --help` for more information.

## Performance

frep is fast. Compared against other tools that respect ignore files such as `.gitignore`, it is the fastest in many scenarios. Here is a benchmark for comparison, performing a find and replace across the entire [Linux kernel repo](https://github.com/torvalds/linux), finding and replacing the string "before" with "after":

<!-- BENCHMARK START -->
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `frep` | 3.772 ± 0.080 | 3.671 | 3.890 | 1.00 |
| `ripgrep + sd` | 3.951 ± 0.196 | 3.732 | 4.206 | 1.05 ± 0.06 |
| `fd + sd` | 10.028 ± 0.002 | 10.025 | 10.029 | 2.66 ± 0.06 |

<!-- BENCHMARK END -->
