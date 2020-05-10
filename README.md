# Activity graph
Visualizes your commit activity in the git repositories found in a
given set of directories.

This program has 3 general use-cases:

1. Printing out a nice visualization of your commits to stdout.

   ```
   activity-graph -r <dir-with-your-repos> -s
   ```

2. Generating a html file to be looked at / served via a file server.

   ```
   activity-graph -r <dir-with-your-repos> -o test.html [-c test.css]
   ```

3. Serving the generated html and css straight from ram via
   [`hyper`][hyper]:

   ```
   activity-graph -r <dir-with-your-repos> server --host 0.0.0.0:80
   ```

## Optional features

- `rayon` is *enabled* by default, but is optional. It allows for the
  parallellization of the underlying `git` commands, which causes a
  ~4x speedup on my system.

- `server` is *disabled* by default, and can be enabled to allow for
  running with the `--server` flag. This causes the program to stay
  alive until manual termination (Ctrl+C), serving the generated HTML
  on a configurable port and address (`--host`). The responses are
  always from a fast cache, and hits to the cache will cause the html
  to be regenerated depending on the `--cache-lifetime` parameter.

## License
I recommend writing your own, it's a fun little project. But even
though I would not recommend using this code, you may use it under the
terms of the [GNU GPLv3 license][license].

[hyper]: https://crates.io/crates/hyper "A fast HTTP 1/2 server written in Rust"
[license]: LICENSE.md "The GNU GPLv3 license text in Markdown."
