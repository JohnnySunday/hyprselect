# hyprselect - Visually focus windows by label

[![CI](https://github.com/K-REBO/hyprselect/workflows/CI/badge.svg)](https://github.com/K-REBO/hyprselect/actions)
[![license](http://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/K-REBO/hyprselect/blob/master/LICENSE)
[![Stars](https://img.shields.io/github/stars/K-REBO/hyprselect.svg)](https://github.com/K-REBO/hyprselect/stargazers)

> **Fork of [svenstaro/wmfocus](https://github.com/svenstaro/wmfocus)** with active Wayland (Hyprland) support.

This tool allows you to rapidly choose a specific window directly without having to use the mouse or directional keyboard navigation.

**Supported window managers:**
- **Hyprland** (Wayland) — actively maintained in this fork, **requires Hyprland 0.56 or newer** (see [Hyprland compatibility](#hyprland-compatibility))
- i3

<video src="https://github.com/user-attachments/assets/9c5aeac1-f342-4704-ba78-18c4ed5cc5be" autoplay loop muted playsinline></video>

Thanks to cairo, it should work on all kinds of screens and automatically display at the correct size according to your DPI.


## Hyprland compatibility

**The Hyprland backend requires Hyprland 0.56 or newer.** For Hyprland 0.55 and older,
use hyprselect 0.1.0.

Hyprland 0.55 deprecated hyprlang in favour of Lua, and as of 0.56 the payload of a
`dispatch` IPC request is evaluated as Lua (`return hl.dispatch(<payload>)`). The old
string form that every pre-0.56 client sends, e.g.

```
dispatch focuswindow address:0x55a1b2c3d4e5
```

is therefore a Lua syntax error and Hyprland answers with
`')' expected near 'address'`. hyprselect now sends the Lua dispatcher expression
instead:

```
dispatch hl.dsp.focus({ window = "address:0x55a1b2c3d4e5" })
```

The [`hyprland`](https://crates.io/crates/hyprland) crate (0.4.0-beta.3, still the
latest release) only knows the old syntax, so hyprselect builds the payload itself and
passes it through the crate's `DispatchType::Custom` escape hatch. Window and monitor
queries are unaffected by the Lua change and keep using the crate as is — only the focus
dispatch had to change, which is why on Hyprland 0.56 the hint overlay of older
hyprselect versions still shows up but pressing a hint key never switches windows.

## Installation

**Pre-built binaries** (no system dependencies required):

Download from the [latest release](https://github.com/K-REBO/hyprselect/releases/latest):

```bash
# Hyprland
curl -L https://github.com/K-REBO/hyprselect/releases/latest/download/hyprselect-hyprland-x86_64-linux -o hyprselect
chmod +x hyprselect
sudo mv hyprselect /usr/local/bin/

# i3
curl -L https://github.com/K-REBO/hyprselect/releases/latest/download/hyprselect-i3-x86_64-linux -o hyprselect
chmod +x hyprselect
sudo mv hyprselect /usr/local/bin/
```

**With Cargo**:

First, install the required system libraries:

```bash
# Hyprland (Wayland)
sudo apt install libcairo2-dev libxcb1-dev libglib2.0-dev libwayland-dev  # Debian/Ubuntu
sudo pacman -S cairo libxcb glib2 wayland                                  # Arch Linux

# i3 (X11)
sudo apt install libcairo2-dev libxcb1-dev libglib2.0-dev libxkbcommon-dev libxkbcommon-x11-dev  # Debian/Ubuntu
sudo pacman -S cairo libxcb glib2 libxkbcommon                                                    # Arch Linux
```

Then install hyprselect:

```bash
cargo install hyprselect --features hyprland  # Hyprland
cargo install hyprselect --features i3        # i3
```

**With Nix** (flakes):
```bash
# Run directly (Hyprland version, default)
nix run github:K-REBO/hyprselect

# Run i3 version
nix run github:K-REBO/hyprselect#hyprselect-i3

# Install to profile
nix profile install github:K-REBO/hyprselect        # Hyprland
nix profile install github:K-REBO/hyprselect#hyprselect-i3  # i3
```

For NixOS/home-manager, add the overlay:
```nix
{
  inputs.hyprselect.url = "github:K-REBO/hyprselect";

  # In your configuration:
  nixpkgs.overlays = [ inputs.hyprselect.overlays.default ];
  environment.systemPackages = [ pkgs.hyprselect ];  # or pkgs.hyprselect-i3
}
```

## Usage

Draw labels on the upper-left corner of all windows:

    hyprselect

Completely fill out windows and draw the label in the middle (try it with transparency!):

    hyprselect --fill

Use a different font (as provided by fontconfig):

    hyprselect -f "Droid Sans":100

Change up the default colors:

    hyprselect --textcolor red --textcoloralt "#eeeeee" --bgcolor "rgba(50, 50, 200, 0.5)"

Filter which Hyprland windows receive hints (repeatable, combined with AND by default, or OR with `--filter-mode any` / `--filter-any`, prefix with `!` to exclude):

    hyprselect --filter class:firefox
    hyprselect -F 'class:kitty' -F '!title:scratchpad'
    hyprselect -F 'floating:false' -F 'monitor:eDP-1'
    hyprselect --filter-mode any -F pid:1234 -F pid:5678 -F pid:9012
    hyprselect --filter-any -F pid:1234 -F pid:5678 -F pid:9012

hyprselect will make use of a compositor to get real transparency.

## Full help
```
Visually focus windows by label

Usage: hyprselect [OPTIONS]

Options:
  -f, --font <FONT>
          Use a specific TrueType font with this format: family:size [default: Mono:72]
  -c, --chars <HINT_CHARS>
          Define a set of possbile values to use as hint characters [default: sadfjklewcmpgh]
  -m, --margin <MARGIN>
          Add an additional margin around the text box (value is a factor of the box size) Format:
          single value or left,right,top,bottom [default: 0.2]
  -p, --print-only
          Print the window id only but don't change focus
  -o, --offset <OFFSET>
          Offset box from edge of window relative to alignment (x,y) [default: 0,0]
  -e, --exit-keys <EXIT_KEYS>
          List of keys to exit application, sequences separator is space, key separator is '+', eg
          Control_L+g Shift_L+f
  -s, --swap
          If this flag is set, the currently active window will swap with the selected window
  -F, --filter <[!]PROP:VALUE>
          Filter Hyprland windows by property (format: [!]<property>:<value>, repeatable). Supported
          properties: class, initialClass, title, initialTitle, address, workspace, monitor, pid,
          tags, floating, fullscreen, pinned, xwayland
      --filter-mode <FILTER_MODE>
          How multiple --filter flags are combined ('all'/'and' or 'any'/'or') [default: all]
          [possible values: all, any]
      --filter-any
          Match windows that satisfy any --filter flag (shorthand for --filter-mode any)
      --textcolor <TEXT_COLOR>
          Text color (CSS notation) [default: #dddddd]
      --textcoloralt <TEXT_COLOR_ALT>
          Text color alternate (CSS notation) [default: #666666]
      --bgcolor <BG_COLOR>
          Background color (CSS notation) [default: "rgba(30, 30, 30, 0.9)"]
      --textcolorcurrent <TEXT_COLOR_CURRENT>
          Text color current window (CSS notation) [default: #333333]
      --textcolorcurrentalt <TEXT_COLOR_CURRENT_ALT>
          Text color current window alternate (CSS notation) [default: #999999]
      --bgcolorcurrent <BG_COLOR_CURRENT>
          Background color current window (CSS notation) [default: "rgba(200, 200, 200, 0.9)"]
      --halign <HORIZONTAL_ALIGN>
          Horizontal alignment of the box inside the window [default: left] [possible values: left,
          center, right]
      --valign <VERTICAL_ALIGN>
          Vertical alignment of the box inside the window [default: top] [possible values: top,
          center, bottom]
      --fill
          Completely fill out windows
  -h, --help
          Print help
  -V, --version
          Print version
```

## Troubleshooting

If there's some funky stuff, you can try to track it down by running `hyprselect` with `RUST_LOG=trace`:

    RUST_LOG=trace hyprselect

This will print quite some useful debugging info.

On Hyprland, if the hint overlay shows up but pressing a hint key does not switch
windows, check the `Dispatched ...` debug line for Hyprland's reply. A Lua error there
means your Hyprland is too old or too new for this version of hyprselect — see
[Hyprland compatibility](#hyprland-compatibility).


## Compiling

**For Hyprland**: You need to have recent versions of `rust`, `cargo`, `wayland-client`, `libxkbcommon` and `cairo` installed.

    git clone https://github.com/K-REBO/hyprselect.git
    cd hyprselect
    cargo run --features hyprland

**For i3**: You need to have recent versions of `rust`, `cargo`, `xcb-util-keysyms`, `libxkbcommon-x11` and `cairo` installed.

    git clone https://github.com/K-REBO/hyprselect.git
    cd hyprselect
    cargo run --features i3


## Contributing

If you want to implement support for more window managers, have a look at the [i3 implementation](https://github.com/K-REBO/hyprselect/blob/master/src/wm_i3.rs) or the [Hyprland implementation](https://github.com/K-REBO/hyprselect/blob/master/src/wm_hyprland.rs).

This tool is heavily inspired by [i3-easyfocus](https://github.com/cornerman/i3-easyfocus).
