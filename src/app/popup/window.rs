//! The popup window: a borderless `tao` window with the Windows 11 acrylic
//! backdrop (DWM), drawn with Direct2D and DirectWrite.

#[cfg(windows)]
pub use imp::Popup;
#[cfg(not(windows))]
pub use stub::Popup;

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use anyhow::{Context, Result};

    use tao::event_loop::EventLoopWindowTarget;
    use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
    use tao::window::{Window, WindowBuilder, WindowId};
    use tracing::{debug, warn};
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Direct2D::Common::{
        D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
    };
    use windows::Win32::Graphics::Direct2D::{
        D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
        D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE,
        D2D1_RENDER_TARGET_PROPERTIES, D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1Factory,
        ID2D1HwndRenderTarget, ID2D1RenderTarget,
    };
    use windows::Win32::Graphics::DirectWrite::{
        DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
        DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_CENTER,
        DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
        DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
    };
    use windows::Win32::Graphics::Dwm::{
        DWM_SYSTEMBACKDROP_TYPE, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_EXTENDED_FRAME_BOUNDS,
        DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmGetWindowAttribute, DwmSetWindowAttribute,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::Win32::UI::Controls::MARGINS;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowPos,
    };
    use windows::core::{PCWSTR, w};

    use crate::app::icon::{COLOR_HIGH, COLOR_LOW, COLOR_MID};
    use crate::app::popup::layout::{Layout, RectF, WIDTH, layout};
    use crate::app::popup::model::{PopupModel, RECORDING_TEXT, ShortcutRow};
    use crate::app::popup::position::{PxRect, popup_origin};
    use crate::app::popup::theme::{Palette, Rgba, is_light, palette, transparency_enabled};
    use crate::app::status::Level;

    /// Distance between the popup and the taskbar or screen edge, in DIP.
    const GAP_DIP: f32 = 12.0;
    /// `D2DERR_RECREATE_TARGET`: the device was lost, build a new target.
    const D2DERR_RECREATE_TARGET: i32 = 0x8899_000C_u32 as i32;

    struct Fonts {
        title: IDWriteTextFormat,
        source: IDWriteTextFormat,
        body: IDWriteTextFormat,
        body_right: IDWriteTextFormat,
        small: IDWriteTextFormat,
        button: IDWriteTextFormat,
    }

    pub struct Popup {
        window: Window,
        hwnd: HWND,
        factory: ID2D1Factory,
        target: Option<ID2D1HwndRenderTarget>,
        fonts: Fonts,
        model: PopupModel,
        layout: Layout,
        light: bool,
        acrylic: bool,
        visible: bool,
        hover: bool,
        hover_change: bool,
        last_click: (i32, i32),
    }

    impl Popup {
        pub fn new<T: 'static>(
            target: &EventLoopWindowTarget<T>,
            model: PopupModel,
        ) -> Result<Self> {
            let layout = layout(&model);
            let window = WindowBuilder::new()
                .with_title("Earbuds")
                .with_decorations(false)
                .with_resizable(false)
                .with_always_on_top(true)
                .with_skip_taskbar(true)
                .with_visible(false)
                .with_inner_size(tao::dpi::LogicalSize::new(
                    WIDTH as f64,
                    layout.height as f64,
                ))
                .build(target)
                .context("Failed to create popup window")?;
            let hwnd = HWND(window.hwnd() as *mut c_void);

            unsafe {
                let corner = DWMWCP_ROUND;
                set_dwm_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner);
                let margins = MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                };
                if let Err(e) = DwmExtendFrameIntoClientArea(hwnd, &margins) {
                    warn!("Popup: DwmExtendFrameIntoClientArea failed: {e}");
                }
            }

            let factory: ID2D1Factory =
                unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
                    .context("Failed to create Direct2D factory")?;
            let fonts = make_fonts().context("Failed to create DirectWrite text formats")?;

            Ok(Self {
                window,
                hwnd,
                factory,
                target: None,
                fonts,
                model,
                layout,
                light: false,
                acrylic: false,
                visible: false,
                hover: false,
                hover_change: false,
                last_click: (0, 0),
            })
        }

        pub fn window_id(&self) -> WindowId {
            self.window.id()
        }

        pub fn is_visible(&self) -> bool {
            self.visible
        }

        /// New content. Redraws if visible, and moves the window if the
        /// height changed, so it stays anchored at the taskbar.
        pub fn set_model(&mut self, model: PopupModel) {
            if model == self.model {
                return;
            }
            let new_layout = layout(&model);
            let height_changed = new_layout.height != self.layout.height;
            self.model = model;
            self.layout = new_layout;
            if !self.model.button.enabled {
                self.hover = false;
            }
            if self.layout.shortcut.and_then(|s| s.change).is_none() {
                self.hover_change = false;
            }
            debug!(
                "Popup set_model: visible={} height_changed={height_changed} status={:?} rows={}",
                self.visible,
                self.model.status_line,
                self.model.rows.len()
            );
            if self.visible {
                if height_changed {
                    self.place();
                }
                self.window.request_redraw();
            }
        }

        /// Shows the popup next to the taskbar, near `click` (physical
        /// screen pixels).
        pub fn show(&mut self, click: (i32, i32)) {
            self.light = is_light(read_personalize_dword(w!("SystemUsesLightTheme")));
            let want_acrylic =
                transparency_enabled(read_personalize_dword(w!("EnableTransparency")));
            unsafe {
                let dark: i32 = if self.light { 0 } else { 1 };
                set_dwm_attr(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark);
                let kind: DWM_SYSTEMBACKDROP_TYPE = if want_acrylic {
                    DWMSBT_TRANSIENTWINDOW
                } else {
                    DWMSBT_NONE
                };
                // Fails before Windows 11 22H2: then we paint a solid background.
                self.acrylic = want_acrylic
                    && DwmSetWindowAttribute(
                        self.hwnd,
                        DWMWA_SYSTEMBACKDROP_TYPE,
                        &kind as *const _ as *const c_void,
                        std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
                    )
                    .is_ok();
            }
            debug!(
                "Popup: show at {click:?} (light: {}, acrylic: {})",
                self.light, self.acrylic
            );
            self.last_click = click;
            self.hover = false;
            self.place();
            self.log_geometry("before set_visible");
            self.window.set_visible(true);
            self.window.set_focus();
            self.visible = true;
            self.log_geometry("after set_visible");
            self.window.request_redraw();
        }

        pub fn hide(&mut self) {
            if self.visible {
                self.window.set_visible(false);
                self.visible = false;
                self.hover = false;
            }
        }

        /// Cursor moved inside the popup (physical client pixels).
        pub fn on_cursor_moved(&mut self, x: f64, y: f64) {
            let scale = self.window.scale_factor();
            let (dx, dy) = ((x / scale) as f32, (y / scale) as f32);
            let over = self.model.button.enabled && self.layout.hit_button(dx, dy);
            let over_change = self.layout.hit_change(dx, dy);
            if over != self.hover || over_change != self.hover_change {
                self.hover = over;
                self.hover_change = over_change;
                self.window.request_redraw();
            }
        }

        pub fn on_cursor_left(&mut self) {
            if self.hover || self.hover_change {
                self.hover = false;
                self.hover_change = false;
                self.window.request_redraw();
            }
        }

        /// True when a left click now would hit the Change link.
        pub fn change_hovered(&self) -> bool {
            self.visible && self.hover_change
        }

        /// True when a left click now would press the (enabled) button.
        pub fn button_hovered(&self) -> bool {
            self.visible && self.hover && self.model.button.enabled
        }

        pub fn request_redraw(&self) {
            self.window.request_redraw();
        }

        /// Sizes and moves the window for the current layout and last click.
        fn place(&mut self) {
            let click = POINT {
                x: self.last_click.0,
                y: self.last_click.1,
            };
            unsafe {
                let monitor = MonitorFromPoint(click, MONITOR_DEFAULTTONEAREST);
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                    warn!("Popup: GetMonitorInfoW failed, not moving the popup");
                    return;
                }
                let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
                if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err() {
                    dpi_x = GetDpiForWindow(self.hwnd).max(96);
                }
                let scale = dpi_x as f32 / 96.0;
                let w = (WIDTH * scale).ceil() as i32;
                let h = (self.layout.height * scale).ceil() as i32;
                let gap = (GAP_DIP * scale).round() as i32;

                // Not `tao::Window::set_inner_size`: on a window that was never
                // shown, tao still counts a caption bar it removes later, and
                // the popup ends up 30 px too tall. Instead force the frame
                // calculation, measure the real non-client size and set the
                // outer rect ourselves.
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
                let (nc_w, nc_h) = non_client_size(self.hwnd);
                // The outer rect includes an invisible resize border. The DWM
                // frame bounds are the visible part; shift by the difference.
                let (dx, dy) = frame_offset(self.hwnd);
                let (x, y) = popup_origin(
                    (click.x, click.y),
                    px_rect(info.rcMonitor),
                    px_rect(info.rcWork),
                    (w, h),
                    gap,
                );
                debug!(
                    "Popup place: click={:?} size={w}x{h} origin=({x},{y}) offset=({dx},{dy}) nc=({nc_w},{nc_h})",
                    self.last_click
                );
                if let Err(e) = SetWindowPos(
                    self.hwnd,
                    None,
                    x - dx,
                    y - dy,
                    w + nc_w,
                    h + nc_h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                ) {
                    warn!("Popup: SetWindowPos failed: {e}");
                }
                self.log_geometry("after place");
            }
        }

        pub fn paint(&mut self) {
            if let Err(e) = self.try_paint() {
                if e.code().0 == D2DERR_RECREATE_TARGET {
                    debug!("Popup: render target lost, recreating");
                    self.target = None;
                    self.window.request_redraw();
                } else {
                    warn!("Popup: paint failed: {e}");
                }
            }
        }

        /// Visible bounds (DWM frame) in screen pixels: left, top, right, bottom.
        pub fn frame_bounds(&self) -> (i32, i32, i32, i32) {
            let mut eb = RECT::default();
            unsafe {
                let _ = DwmGetWindowAttribute(
                    self.hwnd,
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    &mut eb as *mut RECT as *mut c_void,
                    std::mem::size_of::<RECT>() as u32,
                );
            }
            (eb.left, eb.top, eb.right, eb.bottom)
        }

        /// Diagnostics (Phase 11 placement bug).
        pub fn log_geometry(&self, tag: &str) {
            let mut wr = RECT::default();
            let mut cr = RECT::default();
            let mut eb = RECT::default();
            unsafe {
                let _ = GetWindowRect(self.hwnd, &mut wr);
                let _ = GetClientRect(self.hwnd, &mut cr);
                let _ = DwmGetWindowAttribute(
                    self.hwnd,
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    &mut eb as *mut RECT as *mut c_void,
                    std::mem::size_of::<RECT>() as u32,
                );
            }
            debug!(
                "Popup geometry [{tag}]: window=({},{})-({},{}) frame=({},{})-({},{}) client={}x{} layout_h={}",
                wr.left,
                wr.top,
                wr.right,
                wr.bottom,
                eb.left,
                eb.top,
                eb.right,
                eb.bottom,
                cr.right,
                cr.bottom,
                self.layout.height
            );
        }

        fn try_paint(&mut self) -> windows::core::Result<()> {
            let mut rc = RECT::default();
            unsafe { GetClientRect(self.hwnd, &mut rc)? };
            debug!(
                "Popup paint: client={}x{} status={:?} rows={} button={:?}",
                rc.right - rc.left,
                rc.bottom - rc.top,
                self.model.status_line,
                self.model.rows.len(),
                self.model.button.label
            );
            let size = D2D_SIZE_U {
                width: (rc.right - rc.left).max(1) as u32,
                height: (rc.bottom - rc.top).max(1) as u32,
            };
            // tao reports a wrong client size before the first show, so the
            // target is created here and resized whenever the client changes.
            let target = match &self.target {
                Some(t) => {
                    let current = unsafe { t.GetPixelSize() };
                    if current.width != size.width || current.height != size.height {
                        unsafe { t.Resize(&size)? };
                    }
                    t.clone()
                }
                None => {
                    let props = D2D1_RENDER_TARGET_PROPERTIES {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        ..Default::default()
                    };
                    let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                        hwnd: self.hwnd,
                        pixelSize: size,
                        presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                    };
                    let t = unsafe { self.factory.CreateHwndRenderTarget(&props, &hprops)? };
                    self.target = Some(t.clone());
                    t
                }
            };
            let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96) as f32;
            unsafe { target.SetDpi(dpi, dpi) };

            let pal = palette(self.light);
            unsafe {
                target.BeginDraw();
                let bg = if self.acrylic {
                    [0.0, 0.0, 0.0, 0.0]
                } else {
                    pal.background
                };
                target.Clear(Some(&color(bg)));
                let drawn = self.draw_content(&target, &pal);
                let ended = target.EndDraw(None, None);
                drawn?;
                ended
            }
        }

        unsafe fn draw_content(
            &self,
            rt: &ID2D1RenderTarget,
            pal: &Palette,
        ) -> windows::core::Result<()> {
            let m = &self.model;
            let l = &self.layout;
            unsafe {
                text(rt, &m.title, &self.fonts.title, l.title, pal.text)?;
                if let Some(src) = m.source {
                    text(rt, src, &self.fonts.source, l.source, pal.text_secondary)?;
                }
                if let (Some(line), Some(r)) = (m.status_line, l.status_line) {
                    text(rt, line, &self.fonts.body, r, pal.text_secondary)?;
                }
                for (row, r) in m.rows.iter().zip(&l.rows) {
                    text(rt, row.label, &self.fonts.body, r.label, pal.text)?;
                    fill_round(rt, r.bar, 3.0, pal.track)?;
                    if let Some(pct) = row.pct {
                        let mut c = level_color(row.level);
                        if row.stale {
                            c[3] = 0.45;
                        }
                        let fill_right =
                            r.bar.left + (r.bar.right - r.bar.left) * (pct.min(100) as f32 / 100.0);
                        if fill_right > r.bar.left {
                            let fill = RectF::new(r.bar.left, r.bar.top, fill_right, r.bar.bottom);
                            fill_round(rt, fill, 3.0, c)?;
                        }
                    }
                    let pct_text = row.pct.map_or_else(|| "?".to_string(), |p| format!("{p}%"));
                    let pct_color = if row.stale {
                        pal.text_secondary
                    } else {
                        pal.text
                    };
                    text(rt, &pct_text, &self.fonts.body_right, r.pct, pct_color)?;
                }
                if let (Some(runtime), Some(r)) = (&m.runtime, l.runtime) {
                    text(rt, runtime, &self.fonts.body, r, pal.text_secondary)?;
                }
                if let (Some(note), Some(r)) = (m.note, l.note) {
                    text(rt, note, &self.fonts.small, r, pal.text_secondary)?;
                }
                if let (Some(hint), Some(r)) = (m.hint, l.hint) {
                    text(rt, hint, &self.fonts.small, r, pal.text_secondary)?;
                }

                let fill = if self.hover && m.button.enabled {
                    pal.button_hover
                } else {
                    pal.button
                };
                fill_round(rt, l.button, 4.0, fill)?;
                let border = rt.CreateSolidColorBrush(&color(pal.button_border), None)?;
                rt.DrawRoundedRectangle(&rounded(l.button, 4.0), &border, 1.0, None);
                let label_color = if m.button.enabled {
                    pal.text
                } else {
                    pal.text_secondary
                };
                text(
                    rt,
                    m.button.label,
                    &self.fonts.button,
                    l.button,
                    label_color,
                )?;

                if let (Some(row), Some(r)) = (&m.shortcut, l.shortcut) {
                    match row {
                        ShortcutRow::Idle(value) => {
                            text(
                                rt,
                                "Shortcut",
                                &self.fonts.body,
                                r.label,
                                pal.text_secondary,
                            )?;
                            text(rt, value, &self.fonts.body, r.value, pal.text)?;
                            if let Some(c) = r.change {
                                let link = if self.hover_change {
                                    pal.text
                                } else {
                                    pal.accent
                                };
                                text(rt, "Change", &self.fonts.body_right, c, link)?;
                            }
                        }
                        ShortcutRow::Recording(reason) => {
                            let line = match reason {
                                Some(why) => format!("{why}. Try another (Esc cancels)"),
                                None => RECORDING_TEXT.to_string(),
                            };
                            text(rt, &line, &self.fonts.small, r.text, pal.text)?;
                        }
                    }
                }
            }
            Ok(())
        }
    }

    fn make_fonts() -> windows::core::Result<Fonts> {
        unsafe {
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let text_family = w!("Segoe UI Variable Text");
            let fmt = |family: PCWSTR, weight: DWRITE_FONT_WEIGHT, size: f32| {
                dw.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )
            };

            let title = fmt(
                w!("Segoe UI Variable Display"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                15.0,
            )?;
            title.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            title.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let ellipsis = dw.CreateEllipsisTrimmingSign(&title)?;
            let trimming = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            title.SetTrimming(&trimming, &ellipsis)?;

            let source = fmt(text_family, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?;
            source.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            source.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            let body = fmt(text_family, DWRITE_FONT_WEIGHT_NORMAL, 13.0)?;
            body.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            let body_right = fmt(text_family, DWRITE_FONT_WEIGHT_NORMAL, 13.0)?;
            body_right.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            body_right.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            let small = fmt(text_family, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?;
            small.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;

            let button = fmt(text_family, DWRITE_FONT_WEIGHT_NORMAL, 13.0)?;
            button.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            button.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            Ok(Fonts {
                title,
                source,
                body,
                body_right,
                small,
                button,
            })
        }
    }

    fn color(c: Rgba) -> D2D1_COLOR_F {
        D2D1_COLOR_F {
            r: c[0],
            g: c[1],
            b: c[2],
            a: c[3],
        }
    }

    fn level_color(level: Level) -> Rgba {
        let c = match level {
            Level::High => COLOR_HIGH,
            Level::Mid => COLOR_MID,
            Level::Low => COLOR_LOW,
            Level::Unknown => return [0.5, 0.5, 0.5, 1.0],
        };
        [
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
            1.0,
        ]
    }

    fn d2d_rect(r: RectF) -> D2D_RECT_F {
        D2D_RECT_F {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }

    fn rounded(r: RectF, radius: f32) -> D2D1_ROUNDED_RECT {
        D2D1_ROUNDED_RECT {
            rect: d2d_rect(r),
            radiusX: radius,
            radiusY: radius,
        }
    }

    unsafe fn fill_round(
        rt: &ID2D1RenderTarget,
        r: RectF,
        radius: f32,
        c: Rgba,
    ) -> windows::core::Result<()> {
        unsafe {
            let brush = rt.CreateSolidColorBrush(&color(c), None)?;
            rt.FillRoundedRectangle(&rounded(r, radius), &brush);
        }
        Ok(())
    }

    unsafe fn text(
        rt: &ID2D1RenderTarget,
        s: &str,
        format: &IDWriteTextFormat,
        r: RectF,
        c: Rgba,
    ) -> windows::core::Result<()> {
        let utf16: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let brush = rt.CreateSolidColorBrush(&color(c), None)?;
            rt.DrawText(
                &utf16,
                format,
                &d2d_rect(r),
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
    }

    unsafe fn set_dwm_attr<T>(
        hwnd: HWND,
        attr: windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE,
        value: &T,
    ) {
        let res = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attr,
                value as *const T as *const c_void,
                std::mem::size_of::<T>() as u32,
            )
        };
        if let Err(e) = res {
            debug!("Popup: DwmSetWindowAttribute({}) failed: {e}", attr.0);
        }
    }

    /// Outer size minus client size (width, height).
    fn non_client_size(hwnd: HWND) -> (i32, i32) {
        let mut wr = RECT::default();
        let mut cr = RECT::default();
        unsafe {
            if GetWindowRect(hwnd, &mut wr).is_err() || GetClientRect(hwnd, &mut cr).is_err() {
                return (0, 0);
            }
        }
        (
            (wr.right - wr.left) - (cr.right - cr.left),
            (wr.bottom - wr.top) - (cr.bottom - cr.top),
        )
    }

    /// Offset from the outer window rect to the visible DWM frame.
    fn frame_offset(hwnd: HWND) -> (i32, i32) {
        let mut wr = RECT::default();
        let mut eb = RECT::default();
        unsafe {
            if GetWindowRect(hwnd, &mut wr).is_err() {
                return (0, 0);
            }
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut eb as *mut RECT as *mut c_void,
                std::mem::size_of::<RECT>() as u32,
            )
            .is_err()
            {
                return (0, 0);
            }
        }
        (eb.left - wr.left, eb.top - wr.top)
    }

    fn px_rect(r: RECT) -> PxRect {
        PxRect::new(r.left, r.top, r.right, r.bottom)
    }

    /// Reads a DWORD under `HKCU\...\Themes\Personalize`. Read only.
    fn read_personalize_dword(name: PCWSTR) -> Option<u32> {
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let res = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                name,
                RRF_RT_REG_DWORD,
                None,
                Some(&mut value as *mut u32 as *mut c_void),
                Some(&mut size),
            )
        };
        res.is_ok().then_some(value)
    }
}

#[cfg(not(windows))]
mod stub {
    use anyhow::{Result, bail};
    use tao::event_loop::EventLoopWindowTarget;
    use tao::window::WindowId;

    use crate::app::popup::model::PopupModel;

    pub struct Popup;

    impl Popup {
        pub fn new<T: 'static>(_: &EventLoopWindowTarget<T>, _: PopupModel) -> Result<Self> {
            bail!("The popup is only supported on Windows");
        }
        pub fn window_id(&self) -> WindowId {
            unreachable!()
        }
        pub fn is_visible(&self) -> bool {
            false
        }
        pub fn set_model(&mut self, _: PopupModel) {}
        pub fn show(&mut self, _: (i32, i32)) {}
        pub fn hide(&mut self) {}
        pub fn on_cursor_moved(&mut self, _: f64, _: f64) {}
        pub fn on_cursor_left(&mut self) {}
        pub fn button_hovered(&self) -> bool {
            false
        }
        pub fn change_hovered(&self) -> bool {
            false
        }
        pub fn request_redraw(&self) {}
        pub fn paint(&mut self) {}
        pub fn frame_bounds(&self) -> (i32, i32, i32, i32) {
            (0, 0, 0, 0)
        }
    }
}
