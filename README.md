# Blot CLI
**CLI for the [Hack Club Blot](https://blot.hackclub.com/)**

Install it from the [latest release](https://github.com/polypixeldev/blot-cli/releases) or from [crates.io](https://crates.io/crates/hackclub-blot):
```sh
cargo install hackclub-blot
```

**Demo:** https://asciinema.org/a/696324

## About

Blot is a drawing machine made by Hack Club. You can program it using JavaScript in the [Blot editor](https://blot.hackclub.com/editor) online.
However, the controls on the online editor felt a little lacking, and I wanted to be able to control Blot from my headless Raspberry Pi. So I made a CLI!

## Usage

```
CLI for the Hack Club Blot

Usage: blot [OPTIONS] <COMMAND>

Commands:
  go           Move the pen to the specified coordinates
  motors       Manage the Blot's stepper motors
  origin       Manage the Blot's origin
  pen          Manage the Blot's pen
  interactive  Enter interactive mode
  help         Print this message or the help of the given subcommand(s)

Options:
  -p, --port <PORT>  
  -h, --help         Print help
  -V, --version      Print version
```

The Blot CLI features standalone commands for each function that the stock Blot firmware supports. It also has an _interactive mode_, where you can use a simple TUI to control the Blot.
