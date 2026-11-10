# UI style guide (GTK4 + libadwaita)

Appnux uses stock Adwaita widgets and colours, and follows the desktop's accent colour (Yaru orange on Ubuntu, blue on GNOME). Custom styles live in `app/src/style.css`, only for what Adwaita has no class for. Shared building blocks are in `app/src/ui.rs`.

## Page layout

Every page is built with `ui::page(toolbar, content, scroll, footer)`:

| Part | What | Notes |
|---|---|---|
| Header bar | Page title, window buttons | No actions here |
| Toolbar | Filters on the left, actions on the right (Refresh, Scan) | `ui::tool_button`: icon + label |
| Content | Cards, boxed lists | 20px from the toolbar |
| Footer | Only when a page has a selection to act on | Summary on the left, primary button on the right |

- **One column for every part:** each part sits in an `adw::Clamp` of `ui::PAGE_MAX` (1200px), with 24px side margins. No page sets its own width.
- **Content** is `card` boxes and `boxed-list` lists with 16px inner padding and 16px between blocks.
- **Sidebar:** pages on top, Settings alone at the bottom.

## Loading and feedback

- Data on its way shows `ui::skeleton` / `ui::skeleton_row` in the shape of what will appear. A `gtk::Spinner` is only for small inline places.
- Results of an action show as an `adw::Toast`. A question before something that changes the disk is a `ui::dialog`, where Cancel has the focus.
- Warnings and notes are `ui::callout(kind, text)`. Text wraps anywhere, so long paths never overflow.

## Interaction

- Everything clickable gets `ui::pointer`. GTK has no CSS cursor, so it is set per widget.
- The title of an activatable row turns the accent colour on hover (`style.css`).
- Numbers use the `numeric` class and fixed-width columns (`ui::value`).

## Colour for data

The disk bar uses the validated palette in `ui.rs` (`series`), with separate light and dark steps. The legend always prints each part's name, amount and share, so colour is never the only key.

## Checking a change

Run `APPNUX_SNAPSHOT_THEME=light APPNUX_SNAPSHOT_DIR=/tmp/shots ./target/release/appnux`, then the same with `dark`. The app renders every page to PNG and quits; Wayland lets no outside tool capture the window.
