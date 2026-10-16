# Sketcher

A fast pen-sketching app for Windows. Your pages are ordinary files in ordinary folders on your disk: vector ink, text boxes and pictures that stay editable. Existing **Microsoft OneNote** pages can be imported, and their ink becomes strokes you can keep editing. Sketcher never drives the OneNote app.

## Features

- **Hebrew first, right to left.** The interface is in Hebrew with a mirrored right-to-left layout. One click switches to English (left to right). Hebrew and English can be mixed in names, tags, text boxes and search.
- **Pen input with pressure.** Windows Ink pens report pressure, and it changes the stroke width; the pen's eraser button also erases. While the pen touches the screen, touches from your palm are ignored. Every input sample is processed (coalesced pointer events), so fast strokes stay smooth. Touch pans and pinch-zooms by default; drawing with a finger can be turned on in Settings.
- **Pens.** Pick a color from the palette or a full color picker, set the width, and choose a style: Solid, Dashed, Dotted, Highlighter (drawn under the ink) or Pencil.
- **Live diagrams.** With the **Select** tool, tap an item or drag a rectangle around several, then move them, resize them from the corner handle, duplicate or delete them. With something selected, the color, width, style and text-size controls change the selection.
- **Text boxes.** With the **Text** tool, tap anywhere on the page and type. Each paragraph follows its own language direction. Double-tap a text box (Select tool) to edit it; drag its corner to change its width.
- **Pictures.** Add a picture from a file or the photo gallery, take one with the camera, paste one (Ctrl+V), or drop an image file on the page. Large photos are scaled down so page files stay small.
- **Paper.** Blank, Lined, Grid or Dotted paper per page; make a page longer when you need more room.
- **Folders and files.** The sidebar is a tree of folders and pages. Create, rename, move (drag and drop, or *Move to folder…*) and delete pages and folders. Deleted items go to the Recycle Bin.
- **Calculated folder labels.** A folder can keep a fixed name (for example `class of 2021`) and show a label calculated from the date, for example `כיתה ג-3` this school year and `כיתה ד-3` from next September. See below.
- **Tags and search.** Add tags to a page; existing tags are suggested as you type. Search by name and `#tag` together, for example `bridge #work`.
- **Saves automatically,** about a second after you stop drawing. Files are written atomically, so a crash never corrupts a page.
- **Export a page as a PNG image.**
- **Import from OneNote:** a page, a whole section or a whole notebook, into the folder you choose.

### Shortcuts

| Key | Action |
| --- | --- |
| V / P / T / E / H | Select / Pen / Text / Eraser / Pan |
| Delete, Ctrl+D, Ctrl+A | Delete, duplicate, select all |
| Ctrl+Z, Ctrl+Y (or Ctrl+Shift+Z) | Undo, redo |
| Ctrl+V | Paste a picture or text |
| Ctrl+S | Save now |
| Ctrl+N | New page in the selected folder |
| Ctrl+0 | Fit the page width |
| F, Esc | Full page on and off |
| Space + drag | Pan |

## Where your pages live

All pages live under one **page folder**, by default `Documents\Sketcher`. You can choose another one in ⚙ Settings, for example a folder inside OneDrive, and OneDrive will sync it. The folders inside it are your organization; there is no separate index or database, so you can also copy, back up or reorganize the folder in Explorer, and Sketcher picks up the changes.

Each page is one self-contained file, `<page name>.sketcher`, so the page's name is the file's name:

```jsonc
{
  "format": "sketcher/page", "version": 2,
  "id": "…", "tags": ["work"], "paper": "Grid", "width": 794, "height": 1123,
  "strokes": [ { "id": "…", "pen": { "color": {"r":26,"g":115,"b":232,"a":255}, "width": 3, "style": "Solid", "pressure_sensitive": true },
                 "points": [[120.5, 80.25, 0.62], …] } ],          // [x, y, pressure]
  "texts":   [ { "id": "…", "x": 100, "y": 200, "width": 300, "text": "שלום", "size": 20, "color": {…} } ],
  "images":  [ { "id": "…", "x": 0, "y": 0, "width": 320, "height": 200, "src": "data:image/jpeg;base64,…" } ],
  "created": 1760000000000, "modified": 1760000000000,
  "onenote": { … }                                                  // only on imported pages
}
```

Coordinates are in pixels at 96 DPI (an A4 page is 794 × 1123). Ink stays vector data with pressure, never a picture.

A folder with a calculated label also contains a hidden `.sketcher-folder.json`, so the label moves and syncs with the folder.

The first time this version runs, a library from earlier versions (notebooks and sections in `%APPDATA%\Sketcher\data`) is copied into the page folder: each notebook becomes a folder and each section a subfolder.

### Calculated folder labels

Right-click a folder → **Calculated label…**. The only pattern so far is **School class**: choose the grade the class is in *this* school year and its class number, and Sketcher remembers the year the class started first grade. The school year changes on September 1, so the tree shows `כיתה ג-3` until the end of August and `כיתה ד-3` from September. The wording is a template: `{grade}` is the grade letter (א … יב, then `בוגרים` after 12th grade, `גן` before 1st) and `{n}` is the class number. The folder's own name never changes.

## Architecture

Each part of the app is a separate crate. Dependencies point in one direction only.

```
crates/sketch-core    Pure data model: Page (strokes, text boxes, pictures), Pen, tags & search,
                      hit-testing, input smoothing. No I/O, no UI.
crates/sketch-render  Stroke → triangle meshes, stroke → outline → PNG (tiny-skia), paper patterns.
crates/sketch-store   The page folder: tree scan, page files, rename/move/trash, tag index,
                      folder labels, migration from the old library.
crates/onenote        Microsoft Graph: device-code auth, browsing, InkML → strokes, page import.
                      (It can also write pages to OneNote; that isn't exposed in the app yet.)
app/                  Tauri 2 desktop shell (Rust): commands over the crates above,
                      OneNote worker thread, settings.
app/ui/               The interface: plain HTML/CSS/JavaScript with no build step.
                      canvas.js (input, drawing, selection, text editing), tree.js (folders),
                      onenote.js (import panel), media.js (pictures and camera),
                      labels.js (calculated folder labels), export.js (PNG), i18n.js.
```

`sketch-core` doesn't depend on any other crate here. The interface only talks to the backend through Tauri commands, and all paths it sends are checked so they can't leave the page folder.

**Why a web view for the interface?** Rust-native UI toolkits such as egui can't yet lay out right-to-left (bidirectional) text, so Hebrew would render backwards. WebView2 (Chromium, built into Windows 10 and 11) handles Hebrew and RTL fully and supports pen input well.

**Speed.** The page is drawn on two stacked canvases. The base canvas holds the paper, pictures and finished strokes; each stroke is built once as a cached `Path2D`, off-screen strokes are skipped, and the base is redrawn only when you pan, zoom or change the page. The stroke in progress is drawn on a separate canvas in Chromium's low-latency (`desynchronized`) mode. Text boxes are HTML on top, moved with the same transform.

## Importing from OneNote

Click **Import from OneNote**, sign in once, and pick a notebook, section or page. Pages are copied into the folder selected in the tree: a single page goes straight into it, a section into a subfolder named after the section, and a notebook into a folder with one subfolder per section.

- Ink becomes ordinary Sketcher strokes, with color, width, pressure and highlighters kept, so you can keep drawing on, moving or erasing it.
- OneNote is never changed: import is a one-way copy.
- **Your local copy always wins.** Importing a page that was already imported into the same folder keeps the existing file (and any edits you made) and doesn't create a duplicate.
- OneNote's typed text and pictures aren't imported yet, only ink.
- Ink is read through the Graph **beta** endpoint, because that is the only place Microsoft exposes InkML. Right-to-left OneNote pages store ink at negative coordinates; Sketcher moves it onto the page.

## Setting up OneNote access (one-time)

Importing needs a Microsoft sign-in. Microsoft requires every app that uses the OneNote API to have an **app registration**, which gives it an *Application (client) ID*. The registration is free and is only the app's identity. It doesn't decide whose notes you can open: you still sign in with your own account (personal Outlook/Hotmail, or work/school) and see only your own notebooks.

### Quick start: use the project's registration

Sketcher comes with a ready app registration:

```
Application (client) ID: 3f6a4493-f7eb-4548-9e76-53e0e5fc2f84
```

It's used by default, so you can click **Import from OneNote → Sign in** right away. A client ID isn't a secret: apps that sign in this way (public clients) ship their ID inside the program. You still sign in with your own account and approve access yourself, and you only ever see your own notebooks.

The rest of this section is only needed if you want **your own** registration (for example for an organization, or to have the consent screen show your name). Paste that ID into the OneNote panel, or set `SKETCHER_CLIENT_ID`.

### Step 1: Get a directory to register the app in

If you try to register with a personal Microsoft account, you'll probably see this:

> *The ability to create applications outside of a directory has been deprecated. You may get a new directory by joining the M365 Developer Program or signing up for Azure.*

So the app must live in a **directory** (a Microsoft Entra tenant). Pick one of these:

| Option | Who it suits | Notes |
| --- | --- | --- |
| **A. Free Azure account** (recommended for personal use) | Anyone with a personal Microsoft account | Sign up at <https://azure.microsoft.com/free> with your personal account. Microsoft asks for a phone number and a credit card **for identity verification only**. App registrations use the free tier of Entra ID, so nothing is charged. Signing up creates a *Default Directory*, and that's all you need. Students can use **Azure for Students** instead, which needs no card. |
| **B. Your work or school account** | People whose organization lets users register apps (allowed by default) | Sign in to <https://entra.microsoft.com> with your work account. Some organizations require an admin to approve `Notes.ReadWrite`; if so, ask IT or use option A. |
| **C. Microsoft 365 Developer Program sandbox** | Paid Visual Studio Professional/Enterprise subscribers or Microsoft partners only | Since January 2024 the free sandbox is limited to these groups, so personal accounts usually aren't eligible. Not needed for Sketcher. |

### Step 2: Register Sketcher

1. Open <https://entra.microsoft.com>, signed in to the directory from step 1. Check the directory name in the top-right corner.
2. Go to **Entra ID → App registrations → New registration**.
   - **Name:** `Sketcher`
   - **Supported account types:** **Accounts in any organizational directory and personal Microsoft accounts**.
     This setting matters. Your personal account isn't a member of the new directory, so any other choice blocks it with error `AADSTS50020`.
   - **Redirect URI:** leave empty → **Register**.
3. **Authentication** → *Advanced settings* → **Allow public client flows: Yes** → **Save**.
   This turns on the device-code sign-in Sketcher uses. No client secret is needed; don't create one.
4. **API permissions** → **Add a permission** → **Microsoft Graph** → **Delegated permissions** → tick `Notes.ReadWrite` → **Add permissions**.
   `offline_access`, which keeps you signed in, is requested automatically.
   Don't pick the separate **OneNote** tile (`https://onenote.com/`). That's the legacy API, and Sketcher's sign-in tokens are issued for Microsoft Graph.
5. On the **Overview** page, copy the **Application (client) ID**, a GUID like `1a2b3c4d-…`.

### Step 3: Connect Sketcher

1. In Sketcher, click **Import from OneNote** → *Connection (advanced)*, paste the client ID, leave *Tenant* as `common`, and click **Apply**. You can also set the `SKETCHER_CLIENT_ID` environment variable instead.
2. Click **Sign in to OneNote**. Sketcher shows a short code. Click **Open browser**, enter the code, sign in with the account that **owns your notebooks**, and approve the permissions.
   Microsoft may show an "unverified publisher" notice; that's normal for an app you registered yourself.

Sign-in tokens are stored at `%APPDATA%\Sketcher\config\tokens.json` and refresh automatically, so you sign in once.

### Troubleshooting

| Error | Cause | Fix |
| --- | --- | --- |
| `AADSTS50020` … *does not exist in tenant* | The app only accepts accounts from its own directory | Change **Supported account types** to include personal accounts: App registration → Authentication, or edit the manifest's `signInAudience` to `AzureADandPersonalMicrosoftAccount`. |
| `AADSTS7000218` … *client_assertion or client_secret* | Public client flows are off | Step 2.3: **Allow public client flows: Yes**. |
| `AADSTS700016` … *application not found* | Wrong client ID, or a tenant value that doesn't match | Copy the ID again; set Tenant to `common`. |
| `AADSTS65001` / *Need admin approval* | Your organization requires admin consent | Ask your admin, or use a personal account with option A. |
| Personal sign-in works but no notebooks appear | You signed in with a different account than the one that owns the notebooks | **Sign out** in the OneNote panel and sign in with the right account. |

## Building

You need Rust 1.85 or later. On Windows, also install the [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/); it comes preinstalled on Windows 10 and 11. No Node.js or npm is needed.

```powershell
cargo run --release -p sketcher        # run
cargo build --release -p sketcher      # target\release\sketcher.exe
cargo test --workspace
```

To build an installer (.msi or setup.exe), install the Tauri CLI with `cargo install tauri-cli --version "^2"`, then run `cargo tauri build` inside `app/`.

Release builds on Windows don't open a console window. The app also builds and runs on macOS and Linux, which is handy for development.

**Data locations.** Pages are in the page folder (default `Documents\Sketcher`, changeable in Settings; the `SKETCHER_DATA_DIR` environment variable overrides the default). Settings and the OneNote sign-in are in `%APPDATA%\Sketcher\config`.
