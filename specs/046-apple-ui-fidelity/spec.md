# Feature Specification: Apple-Platform UI Fidelity

**Feature Branch**: `046-apple-ui-fidelity`

**Created**: 2026-05-26

**Status**: Draft

---

## Overview

VyomaOS currently renders all UI using an 8×16 bitmap font and flat solid-color rectangles drawn directly to a raw framebuffer. The visual result resembles a high-resolution terminal rather than a modern operating system. This feature upgrades the visual and interaction quality of VyomaOS so that the overall experience feels familiar and polished to users of Apple platforms — macOS, iOS, iPadOS, watchOS, tvOS, and visionOS (spatial computing).

The goal is not a pixel-perfect clone of any Apple OS but a coherent design language: smooth typography, real icons, layered translucent surfaces, fluid animations, and form-factor-aware layouts.

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Readable, Beautiful Typography (Priority: P1)

A user runs VyomaOS and reads text in any app — titles, body text, captions. Text appears crisp at any size rather than blocky bitmap characters. Font sizes scale naturally when the user resizes a window or changes display resolution.

**Why this priority**: Typography is the single highest-leverage change — every element of the UI contains text. Bitmap fonts make the entire OS look unfinished regardless of layout quality.

**Independent Test**: Boot VyomaOS, open the Notes app, type a sentence. Text renders with smooth edges at multiple sizes. Zoom in — no pixel staircase artifacts. Change font size between small/medium/large — all render cleanly.

**Acceptance Scenarios**:

1. **Given** the Notes app is open, **When** the user types text at 14pt, **Then** characters render with smooth anti-aliased edges visible at 2× zoom.
2. **Given** a window title bar, **When** the window is resized, **Then** the title text reflows and remains legible at the new size without artifacts.
3. **Given** a system dialog with small caption text (10pt), **When** it appears on screen, **Then** the text is readable without blurring or pixelation.

---

### User Story 2 — Real App Icons and Images (Priority: P2)

A user looks at the desktop launcher grid and the dock. Each app shows its own unique icon image rather than a text label in a colored box. Icons in the dock animate on hover. The wallpaper can be a photograph or a high-quality gradient image, not just programmatic blocks.

**Why this priority**: Icons are the primary navigation affordance on every Apple platform. Text-label placeholders break the mental model that makes app grids scannable.

**Independent Test**: Boot to desktop. All apps in the icon grid show distinct icons (images, not text boxes). The dock shows the same icons. Clicking an icon launches the app. Screenshot comparison shows visible icon images.

**Acceptance Scenarios**:

1. **Given** the desktop is showing, **When** a user looks at the app grid, **Then** each app cell shows a unique image icon rather than a colored rectangle with text.
2. **Given** an app manifest that declares no icon, **When** the desktop renders, **Then** a default generic icon image is shown (not an error or blank cell).
3. **Given** a wallpaper configured as an image file, **When** the desktop loads, **Then** the wallpaper fills the screen with correct aspect ratio, no stretching.

---

### User Story 3 — Layered Translucent Surfaces (Priority: P3)

A user opens the Spotlight launcher overlay. The background behind the search panel appears blurred and dimmed, similar to the frosted-glass panels on macOS. Windows cast subtle drop shadows onto the desktop below. Panels near the top (menu bar, notifications) feel visually elevated above app content.

**Why this priority**: Alpha compositing is what makes a UI feel "deep" rather than flat. Without it, the z-layer system (already implemented) has no visible effect on perceived depth.

**Independent Test**: Open Spotlight. The panel appears over the desktop with a visible dimmed/blurred background. Move a window — its drop shadow moves with it and overlaps windows beneath it correctly.

**Acceptance Scenarios**:

1. **Given** Spotlight is opened over the desktop, **When** it appears, **Then** the area behind the panel is visibly darker/blurred compared to uncovered areas.
2. **Given** two overlapping windows, **When** the top window is moved, **Then** a soft shadow is cast on the window below, and the shadow moves with the window.
3. **Given** a notification banner appears, **When** it slides in, **Then** it appears above app content with a visible translucent background, not an opaque solid block.

---

### User Story 4 — Polished Window Chrome and Animations (Priority: P3)

A user opens, closes, and minimizes windows. Windows have rounded corners matching macOS proportions. The close/minimize/maximize buttons are color-coded circles (traffic lights). Opening a window plays a brief scale-in animation; closing plays a scale-out fade; minimizing sends the window shrinking to the dock with a genie-like effect.

**Why this priority**: Animations and chrome polish are the tactile layer of the OS — they provide feedback and make transitions feel intentional rather than jarring.

**Independent Test**: Open an app from the dock. Watch the open animation. Click the red close button — window fades and shrinks away. Click the yellow minimize — window collapses toward dock. All transitions complete within 300ms.

**Acceptance Scenarios**:

1. **Given** a window is launching, **When** it appears, **Then** it scales from 80% to 100% with a fade-in over ≤ 300ms.
2. **Given** a window is open, **When** the red close button is clicked, **Then** the window scales down and fades out over ≤ 250ms, then is removed.
3. **Given** a window is open, **When** the yellow minimize button is clicked, **Then** the window animates toward the dock position and disappears into it.
4. **Given** any window, **When** rendered, **Then** corners are rounded (radius ≥ 8px) and a drop shadow is present beneath the frame.

---

### User Story 5 — System UI Components (Priority: P4)

A user clicks the app name in the menu bar. A dropdown menu appears with keyboard-navigable items. Right-clicking on the desktop shows a context menu. A notification banner slides in from the top-right corner with an app icon, title, and body text. The Spotlight overlay background is blurred.

**Why this priority**: These components are the interaction vocabulary of Apple platforms. Without them, the OS layout exists but can't be operated in the familiar way.

**Independent Test**: Click the menu bar. A dropdown appears with at least one item. Press arrow keys to navigate items. Press Escape to dismiss. Right-click desktop — context menu appears with "New Note", "Settings". Dismiss with click-away.

**Acceptance Scenarios**:

1. **Given** the menu bar is showing, **When** the user clicks the active app name, **Then** a dropdown menu panel appears below it with the app's declared menu items.
2. **Given** a dropdown menu is open, **When** the user presses the down arrow key, **Then** focus moves to the next item; pressing Enter activates it.
3. **Given** any app emits a notification, **When** the notification arrives, **Then** a banner slides in from the top-right with the app icon, a bold title, and body text; it auto-dismisses after 4 seconds.
4. **Given** the user right-clicks on the desktop, **When** the context menu appears, **Then** it shows relevant items (New Note, Open Settings, etc.) and dismisses when clicking outside.

---

### User Story 6 — Form Factor Layout Adaptation (Priority: P5)

A user boots VyomaOS with a phone display profile configured. The OS presents a full-screen single-app interface with a swipe-up home gesture, not a windowed desktop. On a watch profile, a single full-screen app occupies the whole display with crown-scroll support. On a TV profile, focusable elements are navigated with arrow keys and a large selection highlight. On visionOS profile, apps float in 3D space and respond to gaze + pinch gestures.

**Why this priority**: Form factor adaptation is what makes "Apple-platform fidelity" meaningful across all six platforms rather than just macOS.

**Independent Test**: Set display profile to `phone`. Boot VyomaOS. No dock or menu bar. One app fills the screen. Swipe up (or press Home key) returns to the app grid. Set profile to `watch`. Single app fills screen. Scroll wheel / up-down keys scroll content.

**Acceptance Scenarios**:

1. **Given** display profile is `desktop`, **When** the OS boots, **Then** dock + menu bar + windowed apps appear in the standard layout.
2. **Given** display profile is `phone`, **When** the OS boots, **Then** no dock or menu bar is visible; one app fills the screen; a home gesture/button reveals the app grid.
3. **Given** display profile is `watch`, **When** the OS boots, **Then** a single app fills the full screen; scroll input scrolls content vertically.
4. **Given** display profile is `tv`, **When** the user presses arrow keys, **Then** a visible focus ring moves between focusable elements; pressing Select/Enter activates the focused element.
5. **Given** display profile is `vision`, **When** apps are open, **Then** windows are positioned as floating panels in a spatial layout; gaze targeting highlights the focused window.

---

### Edge Cases

- What happens when an app declares no icon? → Default generic icon shown; no crash or blank cell.
- What happens when a font file is missing from the initramfs? → Fall back to current bitmap font; log warning; no crash.
- What happens when an image file is corrupt or invalid format? → Render a colored placeholder rectangle; no crash.
- What happens when an animation is requested but the frame rate drops below 30fps? → Skip frames rather than slow the animation; animation always completes in wall-clock time.
- What happens when a notification arrives while another banner is already showing? → Queue the second notification; display after the first auto-dismisses.
- What happens when the display profile is unrecognized? → Fall back to `desktop` profile; log warning.

---

## Functional Requirements *(mandatory)*

### FR-1: Scalable Font Rendering
- The system shall render text at any point size (8pt–96pt) with smooth edges
- Apps shall specify text size in points, not size codes (s/m/l)
- The system shall ship at least one proportional sans-serif typeface and one monospace typeface in the base image
- Text rendering shall support regular, bold, and italic variants

### FR-2: Image and Icon Rendering
- The system shall decode and display raster images (PNG format minimum)
- Each app manifest may declare an `icon` path pointing to a PNG file
- The desktop grid and dock shall display app icons from manifest declarations
- The system shall supply a default icon for apps without a declared icon
- The system shall support a wallpaper configured as an image file path

### FR-3: Alpha Compositing
- The framebuffer compositor shall blend pixel layers using per-pixel RGBA alpha values
- Surfaces with alpha < 255 shall blend with the layers beneath them
- Drop shadows shall be rendered as semi-transparent blur regions beneath window frames
- Frosted-glass / blur-behind effects shall be achievable for overlay panels

### FR-4: Window Chrome Polish
- Window corners shall be rounded with a configurable radius (default ≥ 8px)
- Each window shall show three control buttons (close = red, minimize = yellow, maximize = green) matching macOS proportions (~12px diameter)
- Window open/close/minimize transitions shall animate over ≤ 300ms
- Windows shall cast a soft drop shadow

### FR-5: System UI Components
- The menu bar shall support clickable app-name dropdown menus
- Apps shall be able to declare menu items in their manifests
- The system shall support right-click context menus at any screen position
- The system shall support notification banners: icon + title + body, auto-dismiss after 4 seconds
- The Spotlight overlay shall render a blurred/dimmed background behind its search panel

### FR-6: Form Factor Layout Profiles
- The system shall support six named display profiles: `desktop`, `phone`, `tablet`, `watch`, `tv`, `vision`
- Each profile shall define a distinct shell layout and primary interaction model
- The active profile shall be set at boot time from the display configuration
- Profile-specific UI chrome elements (dock, menu bar, home bar, focus ring, spatial anchors) shall appear only when the matching profile is active

---

## Success Criteria *(mandatory)*

1. **Typography legibility**: A user study (or screenshot comparison) shows text in all apps is readable at sizes 10pt–48pt without visible pixel staircase artifacts.
2. **Icon recognition**: Users can identify all auto-started apps by icon alone (without reading app names) at ≥ 85% accuracy in a recognition test.
3. **Animation smoothness**: All window open/close/minimize animations complete within 300ms at ≥ 30fps with no visible tearing on the target display.
4. **Depth perception**: A blind A/B comparison between the old flat UI and the new composited UI shows ≥ 80% of testers prefer the new version for "feeling modern and polished".
5. **Form factor coverage**: All six profiles boot successfully and present a distinct layout appropriate for the target device class.
6. **Regression-free**: All existing apps continue to launch, display content, and respond to input after the rendering upgrade; no previously working feature breaks.
7. **Boot time preserved**: System boot time to first interactive frame remains under 5 seconds on the reference hardware configuration.

---

## Key Entities *(optional)*

| Entity | Description |
|--------|-------------|
| `DisplayProfile` | Named layout configuration (desktop/phone/tablet/watch/tv/vision) with shell chrome rules |
| `AppIcon` | PNG image file declared in app manifest; cached by supervisor at launch |
| `Typeface` | TTF/OTF font file shipped in initramfs; loaded by supervisor font rasterizer |
| `Layer` | Compositing surface with RGBA pixel buffer and z-index; owned by one window or system chrome element |
| `Shadow` | Pre-computed blur mask attached to a window frame; composited below the window layer |
| `MenuBar` | Top-strip system chrome element holding the active app's declared menu items |
| `DropdownMenu` | Panel rendered on click; contains navigable menu items declared by the active app |
| `ContextMenu` | Right-click panel; position-anchored; dismissed on outside click |
| `NotificationBanner` | Timed overlay panel; icon + title + body; queued; auto-dismisses after 4s |
| `Animation` | Timed transition between two visual states; wall-clock bounded; frame-skipping on overload |

---

## Assumptions *(optional)*

1. The target display resolution for `desktop` profile is 1440×900 or higher; assets are designed for this resolution and up.
2. The font rasterizer will be compiled as part of the supervisor binary (not a WASM app) to avoid the overhead of cross-process glyph rendering.
3. PNG is the only required image format for this spec; JPEG and WebP are explicitly out of scope.
4. The `vision` profile layout is a best-effort spatial approximation on a 2D framebuffer — true depth/parallax requires future GPU work.
5. Animations are CPU-rendered with double-buffering; GPU acceleration is out of scope for this spec.
6. Apple's proprietary San Francisco typeface is not redistributable; VyomaOS will ship an open-licensed alternative (e.g., Inter, Noto Sans) that matches the geometric proportions of the Apple design language.
7. "Frosted glass" blur is approximated by a darkened semi-transparent overlay, not a true Gaussian blur of the layer beneath (true blur requires GPU).

---

## Out of Scope *(optional)*

- GPU acceleration (Metal, Vulkan, OpenGL)
- CoreText-level text shaping, ligatures, kerning tables, RTL text
- Emoji rendering
- Animated GIF or video playback
- Resize handles on windows (separate spec)
- iCloud / Apple ecosystem integration
- Haptic feedback
- Accessibility (screen reader, VoiceOver) — separate spec
