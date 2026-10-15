# Sketcher

A fast, native pen-sketching app written in Rust. You can organize sketches into notebooks, sections and pages, tag and search them, and save them to or open them from **Microsoft OneNote** through the Microsoft Graph API. It never drives the OneNote app itself.

## Features

- **Pen input with pressure.** Windows Ink pens and touch screens report pressure, and it changes the stroke width. A mouse also works. Every input event is processed, not one sample per frame, so fast strokes stay smooth.
- **Pens.** Pick a color from the palette or a full color picker, set the width, and choose a style: Solid, Dashed, Dotted, Highlighter (drawn under the ink) or Pencil. Pressure sensitivity can be turned on or off.
- **Tools.** Pen, a stroke eraser with adjustable size, and Pan (or hold Space, or drag with the middle mouse button). Undo and redo. Zoom with Ctrl+scroll or a pinch gesture.
- **Paper.** Choose Blank, Lined, Grid or Dotted paper per page, and make a page longer when you need more room.
- **Organization.** Notebooks contain sections, and sections contain pages. Rename, delete or add items from the right-click menu.
- **Tags and search.** Add tags to a page; existing tags are suggested as you type. Search by title words and `#tag` together, for example `bridge #work`. Clicking a tag in the tag list filters by it.
- **Local storage.** Changes save automatically and work offline. Files are written atomically, so a crash won't corrupt them.
- **Export a page as a PNG image.**
- **OneNote.** Sign in, browse your notebooks, sections and pages, open a OneNote page and edit it, save a page back, save a whole section, or import a whole section.

### Shortcuts

| Key | Action |
| --- | --- |
| P / E / H | Pen / Eraser / Pan |
| Ctrl+Z, Ctrl+Y (or Ctrl+Shift+Z) | Undo, redo |
| Ctrl+S | Save now |
| Ctrl+N | New page in the current section |
| Ctrl+0 | Fit the page width |
| Space + drag | Pan |

## Architecture

Each part of the app is a separate crate. Dependencies point in one direction only.

```
crates/sketch-core    Pure data model: Page/Stroke/Pen/Library, undo history, tags & search,
                      hit-testing, input smoothing. No I/O, no UI.
crates/sketch-render  Stroke → triangle meshes (for any GPU UI), stroke → outline → PNG (tiny-skia),
                      paper patterns.
crates/sketch-store   Local persistence (library.json + pages/<id>.json).
crates/onenote        Microsoft Graph: device-code auth, browse, create/patch/delete pages,
                      InkML ⇄ strokes, Sketcher page ⇄ OneNote page.
app/                  egui/wgpu desktop UI. OneNote calls run on a background thread.
```

`sketch-core` doesn't depend on any other crate here, and the UI is the only crate that uses egui. That means you could swap the UI or the storage without touching the rest.

**Speed.** Rendering goes through wgpu, which uses Direct3D 12 on Windows. Each stroke is tessellated once in page coordinates and cached. Strokes are projected to the screen again only when you pan, zoom or change the page. Off-screen strokes are skipped. While you draw, only the stroke in progress is tessellated each frame.

## How OneNote sync works

A page that Sketcher writes to OneNote contains:

1. **Your strokes**, in one of two forms (choose under ⚙ Settings):
   - **Native OneNote ink** (the default). The strokes are real OneNote ink, sent as InkML, so you can keep editing them with OneNote's own pens. Dashed and dotted strokes become one ink stroke per dash or dot.
   - **Picture.** The page is rendered as a PNG image.
2. **A tags line**, for example `Tags: #work #ideas`, which you can read and search in OneNote.
3. **A small attachment** named `*.sketcher.json` holding the exact Sketcher page (pen styles, pressure, paper). This lets a page go to OneNote and back without losing anything.

**Opening a page from OneNote** (OneNote panel → pick a section → Open):

- If the page has the Sketcher attachment and its ink hasn't changed in OneNote, Sketcher loads the attachment and the page comes back exactly as it was.
- If someone edited the ink in OneNote since then, the OneNote ink wins. Sketcher keeps the paper and page settings from the attachment.
- A page Sketcher never wrote is imported from its OneNote ink. Typed text and images on that page aren't imported.

Imported pages go into a local section linked to the OneNote section (marked ☁). Choosing **Save to OneNote** writes the page back to its original OneNote page.

**API limits to know about:**

- The OneNote API can't change ink that is already on a page. So re-saving a page in **ink mode** creates a new page and, if *Replace previous page* is on, deletes the old one. Anything else added to that page in OneNote is lost, and the page moves to the end of the section.
- **Picture mode** doesn't have this problem: the image, attachment, title and tags are updated in place, and other content on the page is kept.
- Ink is read and written through the Graph **beta** endpoint, because that is the only place Microsoft exposes InkML.

## Setting up OneNote access (one-time)

Sketcher signs you in with Microsoft's *device code* flow. You need an app registration, which is free:

1. Go to <https://entra.microsoft.com> → **App registrations** → **New registration**.
   - Name: `Sketcher`
   - Supported account types: **Accounts in any organizational directory and personal Microsoft accounts**. Choose this if you want personal OneNote notebooks to work.
   - Leave the Redirect URI empty → **Register**.
2. **Authentication** → *Advanced settings* → **Allow public client flows: Yes** → Save.
3. **API permissions** → Add → Microsoft Graph → *Delegated* → `Notes.ReadWrite` (`offline_access` is requested automatically).
4. Copy the **Application (client) ID**. In Sketcher, open ☁ OneNote → Connection, paste the ID, and click **Apply**. You can also set the `SKETCHER_CLIENT_ID` environment variable instead.
5. Click **Sign in to OneNote**, open the link shown, enter the code, and approve.

Sign-in tokens are stored at `%APPDATA%\Sketcher\config\tokens.json` and refresh automatically.

## Building

You need Rust 1.85 or later.

```powershell
cargo run --release -p sketcher        # run
cargo build --release -p sketcher      # target\release\sketcher.exe — a single, self-contained exe
cargo test --workspace
```

Release builds on Windows don't open a console window. The app also builds and runs on macOS and Linux, which is handy for development.

**Data locations.** The library is stored in `%APPDATA%\Sketcher\data`. Set `SKETCHER_DATA_DIR` to use a different folder, for example a portable folder or a synced drive.
