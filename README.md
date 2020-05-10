# Activity graph
Visualizes your commit activity in the git repositories found in a
given set of directories.

This program has 3 general use-cases:

1. Printing out a nice visualization of your commits to stdout.

   ```
   activity-graph -r <dir-with-your-repos> -s
   ```

2. Generating a html file (and, optionally, a css file instead of a
   `<style>`) to be looked at / served via a file server.

   ```
   activity-graph -r <dir-with-your-repos> generate -o test.html [-c test.css]
   ```

3. Serving the generated html and css straight from memory via
   [`hyper`][hyper]:

   ```
   activity-graph -r <dir-with-your-repos> server --host 0.0.0.0:80
   ```

## Building

Install Rust 1.43.1 and Cargo 1.43.0 (or newer), and then run the
following command:

```
cargo build --release [--features server]
```

The executable is `target/release/activity-graph[.exe]`.

Might work on older versions of Rust and/or Cargo, and probably does,
but those versions are what I wrote this with.

## Optional features

- `rayon` is *enabled* by default, but is optional. It allows for the
  parallellization of the underlying `git` commands, which causes a
  ~4x speedup on my system.

- `server` is *disabled* by default, and can be enabled to allow for
  the third described usecase, via the `server` subcommand. This
  causes the program to stay alive until manual termination (Ctrl+C),
  serving the generated HTML on a configurable port and address
  (`--host`). The responses are always from a fast cache, and hits to
  the cache will cause the html to be regenerated depending on the
  `--cache-lifetime` parameter.

## License
I recommend writing your own, it's a fun little project. But even
though I would not recommend using this code, you may use it under the
terms of the [GNU GPLv3 license][license].

## Usage

You could just use the `--help` flag, but here you go.

```
activity-graph 0.1.0
Jens Pitkanen <jens@neon.moe>
Generates a visualization of your commit activity in a set of git repositories.

USAGE:
    activity-graph.exe [FLAGS] [OPTIONS] [SUBCOMMAND]

FLAGS:
    -h, --help       Prints help information
    -s, --stdout     Prints a visualization into stdout
    -V, --version    Prints version information
    -v, --verbose    Prints verbose information

OPTIONS:
    -a, --author <author>                      Regex that matches the author(s) whose commits are being counted (if not
                                               set, all commits will be accounted for)
    -d, --depth <depth>                        How many subdirectories deep the program should search (if not set, there
                                               is no limit)
        --external-css <external-css>          A css file that will be pasted at the end of the css
        --external-footer <external-footer>    A html file that will be pasted at the end of the <body> element
        --external-head <external-head>        A html file that will be pasted in the <head> element
        --external-header <external-header>    A html file that will be pasted at the beginning of the <body> element
    -r, --repos <repos>...                     Path(s) to the directory (or directories) containing the repositories you
                                               want to include

SUBCOMMANDS:
    generate    Output the generated html into a file
    help        Prints this message or the help of the given subcommand(s)
    server      Run a server that serves the generated activity graph html
```

The `generate` command:

```
activity-graph-generate 0.1.0
Output the generated html into a file

USAGE:
    activity-graph.exe generate [OPTIONS]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -c, --css <css>      The file that the stylesheet will be printed out to (if not set, it will be included in the
                         html inside a style-element)
    -o, --html <html>    The file that the resulting html will be printed out to [default: activity-graph.html]
```

The `server` command:

```
activity-graph-server 0.1.0
Run a server that serves the generated activity graph html

USAGE:
    activity-graph.exe server [OPTIONS]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --cache-lifetime <cache-lifetime>    The minimum amount of seconds between regenerating the html and css
                                             [default: 1]
        --host <host>                        The address that the server is hosted on [default: 127.0.0.1:80]
```

[hyper]: https://crates.io/crates/hyper "A fast HTTP 1/2 server written in Rust"
[license]: LICENSE.md "The GNU GPLv3 license text in Markdown."
