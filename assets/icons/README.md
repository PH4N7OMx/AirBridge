# AirBridge icon

`AirBridge.png` is the generated master artwork. `AirBridge.ico` contains native
16, 20, 24, 32, 40, 48, 64, 128 and 256 px representations for the executable and UI.
The original generated master is retained unchanged; ICO packaging only resizes it.
The Rust build embeds the ICO, so a deployed EXE does not need external image files.

Created using the built-in imagegen tool for this project. Final generation prompt:

> Polish this exact AirBridge application icon for shipping. Keep the blue tile and
> white smartphone/monitor/wireless artwork visually unchanged. Remove every stray
> cyan/blue pixel, speck and fringe outside the rounded tile. Make the outer silhouette
> a perfectly symmetric clean iOS rounded square with smooth antialiasing and exactly
> equal margins on all sides. Everything beyond that silhouette must be completely
> transparent alpha zero, with no shadow or glow. No other changes. Deliver a centered
> square image, production clean transparency.

The artwork uses no Apple logo or SF Symbols. The app's GPL-3.0-or-later license applies
to this project asset. Inter font licensing is separate in `../fonts/`.
