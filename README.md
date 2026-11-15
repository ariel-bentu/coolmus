# Sketcher

A fast pen-sketching app for Windows. Every page is an ordinary **PDF file** in your own folders: anyone can open it in any PDF viewer, and in Sketcher its vector ink, text boxes and pictures stay fully editable.

## Features

- **Hebrew first, right to left.** The interface is in Hebrew with a mirrored right-to-left layout. One click switches to English (left to right). Hebrew and English can be mixed in names, tags, text boxes and search.
- **Pen input with pressure.** Windows Ink pens report pressure, and it changes the stroke width; the pen's eraser button also erases. While the pen touches the screen, touches from your palm are ignored. Every input sample is processed (coalesced pointer events), so fast strokes stay smooth. Touch pans and pinch-zooms by default; drawing with a finger can be turned on in Settings.
- **Pens.** Pick a color from the palette or a full color picker, set the width, and choose a style: Solid, Dashed, Dotted, Highlighter (drawn under the ink) or Pencil.
- **Live diagrams.** With the **Select** tool, tap an item or drag a rectangle around several, then move them, resize them from the corner handle, duplicate or delete them. With something selected, the color, width, style and text-size controls change the selection.
- **Text boxes.** With the **Text** tool, tap anywhere on the page and type. Each paragraph follows its own language direction. Double-tap a text box (Select tool) to edit it; drag its corner to change its width.
- **Pictures.** Add a picture from a file or the photo gallery, take one with the camera, paste one (Ctrl+V), or drop an image file on the page. Large photos are scaled down so page files stay small.
- **A4 sheets, several per file.** A page file is a stack of A4 sheets; *Add page* in the page menu adds one, and each sheet is a page in the PDF and in print. Ink stays on the paper: strokes start only on a sheet and are cut at its edges, and the page never grows by itself. Scrolling stops at the first sheet's top and the last sheet's bottom, and a page narrower than the window stays centered.
- **Paper.** Blank, Lined, Grid or Dotted paper per page.
- **Your folders, as they are on disk.** Add one or more folders (for example `Documents\Sketcher` and a folder inside OneDrive); the sidebar shows them with their real subfolders and files. Create, rename, move (drag and drop between any of them, or *Move to folder…*) and delete pages and folders. Deleted items go to the Recycle Bin.
- **Copy and paste with File Explorer.** Copy (Ctrl+C) a page or folder in Sketcher and paste it in Explorer, or copy files in Explorer and paste them into a Sketcher folder. Cut (Ctrl+X) and paste within Sketcher moves.
- **Other PDFs** in your folders are listed too, and open in your PDF viewer.
- **Print** a page (Ctrl+P), or all pages of a folder.
- **Import from OneNote files.** Right-click a folder → *Import from OneNote…* and pick a notebook's `Open Notebook.onetoc2` or a single section `.one`. See below.
- **Calculated folder labels.** A folder can keep a fixed name (for example `class of 2021`) and show a label calculated from the date, for example `כיתה ג-3` this school year and `כיתה ד-3` from next September. See below.
- **Tags and search.** Add tags to a page; existing tags are suggested as you type. Search by name and `#tag` together, for example `bridge #work`.
- **Saves automatically,** about a second after you stop drawing. Files are written atomically, so a crash never corrupts a page.
- **Export a page as a PNG image.**

### Shortcuts

| Key | Action |
| --- | --- |
| V / P / T / E / H | Select / Pen / Text / Eraser / Pan |
| Delete, Ctrl+D, Ctrl+A | Delete, duplicate, select all |
| Ctrl+Z, Ctrl+Y (or Ctrl+Shift+Z) | Undo, redo |
| Ctrl+V | Paste a picture or text (on the page) |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste files and folders (after clicking in the tree) |
| Ctrl+P | Print the page |
| Ctrl+S | Save now |
| Ctrl+N | New page in the selected folder |
| Ctrl+0 | Fit the page width |
| F, Esc | Full page on and off |
| Space + drag | Pan |

## Where your pages live

Sketcher shows the folders you add in ⚙ Settings (by default `Documents\Sketcher`), exactly as they are on disk. There is no separate index or database: you can copy, back up or reorganize them in Explorer, and Sketcher picks up the changes. A folder inside OneDrive is synced by OneDrive. Removing a folder from the list doesn't touch it on disk.

Each page is one file, `<page name>.pdf`, so the page's name is the file's name. It is a normal one-page PDF, drawn as vectors: the paper, the ink (pressure-width strokes as filled outlines), pictures, and text boxes as real, selectable and searchable text with the system font embedded. Inside, as a PDF attachment named `sketcher-page.json`, it carries the page itself:

```jsonc
{
  "format": "sketcher/page", "version": 2,
  "id": "…", "tags": ["work"], "paper": "Grid", "width": 794, "height": 1123, "sheets": 2,
  "strokes": [ { "id": "…", "pen": { "color": {"r":26,"g":115,"b":232,"a":255}, "width": 3, "style": "Solid", "pressure_sensitive": true },
                 "points": [[120.5, 80.25, 0.62], …] } ],          // [x, y, pressure]
  "texts":   [ { "id": "…", "x": 100, "y": 200, "width": 300, "text": "שלום", "size": 20, "color": {…} } ],
  "images":  [ { "id": "…", "x": 0, "y": 0, "width": 320, "height": 200, "src": "data:image/jpeg;base64,…" } ],
  "created": 1760000000000, "modified": 1760000000000
}
```

Sketcher reads the attachment back, so nothing is lost between edits; the drawing is regenerated from it on every save. Coordinates are in pixels at 96 DPI. `width` × `height` is one sheet (A4 is 794 × 1123); the sheets are stacked top to bottom 24 units apart, and all content shares that one coordinate space; ink stays vector data with pressure, never a picture.

**Edited in another program?** The page records a fingerprint of what Sketcher drew. If someone adds comments in Acrobat or otherwise changes the page, Sketcher notices when you open it and asks: open it in your PDF viewer (keeping their changes), or edit it in Sketcher (its next save replaces those changes). Re-saving a page unchanged in Preview or Acrobat is fine.

A folder with a calculated label also contains a hidden `.sketcher-folder.json`, so the label moves and syncs with the folder.

### Importing from OneNote

Sketcher reads OneNote's own files, so no Microsoft sign-in is needed. Get them by downloading a notebook from OneDrive (a `.zip` with one `.one` per section and an `Open Notebook.onetoc2`; if the web page offers no download, OneNote for Windows can export a section or notebook too), or use the `.one` files OneNote desktop keeps.

Pick `Open Notebook.onetoc2` to import a whole notebook, or one `.one` file for a single section. The notebook becomes a folder, each section group and section a subfolder, and each page a page file named after its title. OneNote's recycle bin is skipped. The import is a copy: the OneNote files are not changed.

- **Ink** becomes ordinary strokes with its color and width; highlighters stay highlighters. OneNote's files don't keep pen pressure, so imported strokes have an even width.
- **Typed text** becomes text boxes at the same places, with its size and color. Tables become one line per row.
- **Pictures** keep their position and size.
- Right-to-left pages, which OneNote stores at negative positions, are moved onto the page.

### Printing

**Print** (Ctrl+P, the page menu, or right-click a page) and **Print the pages in this folder** (right-click a folder) write one PDF to a temporary folder and open it in your PDF viewer (Edge, unless you chose another). Print from there: the viewer's print dialog has the printer, copies, page range and preview. The printed copy doesn't include the page data.

### Calculated folder labels

Right-click a folder → **Calculated label…**. The only pattern so far is **School class**: choose the grade the class is in *this* school year and its class number, and Sketcher remembers the year the class started first grade. The school year changes on September 1, so the tree shows `כיתה ג-3` until the end of August and `כיתה ד-3` from September. The wording is a template: `{grade}` is the grade letter (א … יב, then `בוגרים` after 12th grade, `גן` before 1st) and `{n}` is the class number. The folder's own name never changes.

## Architecture

Each part of the app is a separate crate. Dependencies point in one direction only.

```
crates/sketch-core    Pure data model: Page (strokes, text boxes, pictures), Pen, tags & search,
                      hit-testing, input smoothing. No I/O, no UI.
crates/sketch-render  Stroke → triangle meshes, stroke → outline → PNG (tiny-skia), paper patterns.
crates/sketch-pdf     Page ⇄ PDF: draws a page as vector PDF (krilla, with bidi text layout),
                      attaches and reads back the page data, detects edits by other programs.
crates/onenote-import OneNote .one / .onetoc2 files → Sketcher pages (via onenote_parser).
crates/sketch-store   The root folders: tree scan, page files, rename/move/copy/trash, tag index,
                      folder labels.
app/                  Tauri 2 desktop shell (Rust): commands over the crates above, system
                      clipboard file lists, printing, settings.
app/ui/               The interface: plain HTML/CSS/JavaScript with no build step.
                      canvas.js (input, drawing, selection, text editing), tree.js (folders),
                      media.js (pictures and camera), labels.js (calculated folder labels),
                      export.js (PNG), i18n.js.
```

`sketch-core` doesn't depend on any other crate here. The interface only talks to the backend through Tauri commands, and all paths it sends are checked so they can't leave the folders you added.

**Why a web view for the interface?** Rust-native UI toolkits such as egui can't yet lay out right-to-left (bidirectional) text, so Hebrew would render backwards. WebView2 (Chromium, built into Windows 10 and 11) handles Hebrew and RTL fully and supports pen input well.

**Speed.** The page is drawn on two stacked canvases. The base canvas holds the paper, pictures and finished strokes; each stroke is built once as a cached `Path2D`, off-screen strokes are skipped, and the base is redrawn only when you pan, zoom or change the page. The stroke in progress is drawn on a separate canvas in Chromium's low-latency (`desynchronized`) mode. Text boxes are HTML on top, moved with the same transform.

## Building

You need Rust 1.85 or later. On Windows, also install the [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/); it comes preinstalled on Windows 10 and 11. No Node.js or npm is needed.

```powershell
cargo run --release -p sketcher        # run
cargo build --release -p sketcher      # target\release\sketcher.exe
cargo test --workspace
```

To build an installer (.msi or setup.exe), install the Tauri CLI with `cargo install tauri-cli --version "^2"`, then run `cargo tauri build` inside `app/`.

Release builds on Windows don't open a console window. The app also builds and runs on macOS and Linux, which is handy for development.

**Data locations.** Pages are in the folders you add (default `Documents\Sketcher`; the `SKETCHER_DATA_DIR` environment variable overrides the default). Settings are in `%APPDATA%\Sketcher\config`.
