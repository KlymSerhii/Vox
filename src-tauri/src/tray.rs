use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

// `Manager` is used to look up the settings window.

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, "open-settings", "Open Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Vox", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &quit_item])?;

    let icon = tray_icon_image(app);

    let mut builder = TrayIconBuilder::with_id("vox-tray")
        .tooltip("Vox — hold hotkey to dictate")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open-settings" => {
                show_settings(app);
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_settings(tray.app_handle());
            }
        });

    if let Some(img) = icon {
        builder = builder.icon(img).icon_as_template(true);
    }

    builder.build(app)?;
    Ok(())
}

fn show_settings<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Return a simple programmatic tray icon. In Phase 1 we skip bundled PNGs
/// and draw a minimal alpha-only "mic" template so the app always boots.
fn tray_icon_image<R: Runtime>(_app: &AppHandle<R>) -> Option<Image<'static>> {
    Some(placeholder_icon())
}

fn placeholder_icon() -> Image<'static> {
    // 16x16 "mic" template: filled column of pixels in the middle.
    // Alpha-only template; channel values are ignored except alpha when
    // `icon_as_template(true)`.
    const W: u32 = 16;
    const H: u32 = 16;
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    for y in 2..13 {
        for x in 6..10 {
            let i = ((y * W + x) * 4) as usize;
            rgba[i] = 0;
            rgba[i + 1] = 0;
            rgba[i + 2] = 0;
            rgba[i + 3] = 255;
        }
    }
    // Stand line
    for y in 12..15 {
        for x in 7..9 {
            let i = ((y * W + x) * 4) as usize;
            rgba[i + 3] = 255;
        }
    }
    Image::new_owned(rgba, W, H)
}
