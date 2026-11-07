pub const AUMID: &str = "EarbudsTray";

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn toast_xml(title: &str, body: &str) -> String {
    let esc_title = xml_escape(title);
    let esc_body = xml_escape(body);
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{esc_title}</text><text>{esc_body}</text></binding></visual></toast>"
    )
}

#[cfg(windows)]
struct SafeHKey(windows::Win32::System::Registry::HKEY);

#[cfg(windows)]
impl Drop for SafeHKey {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = windows::Win32::System::Registry::RegCloseKey(self.0);
            }
        }
    }
}

#[cfg(windows)]
pub fn register_aumid() -> windows::core::Result<()> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCreateKeyExW,
        RegSetValueExW,
    };

    let mut hkey = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            windows::core::w!("Software\\Classes\\AppUserModelId\\EarbudsTray"),
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(windows::core::Error::from(status));
    }
    let safe_key = SafeHKey(hkey);

    let display_name_bytes: Vec<u8> = "Earbuds Tray\0"
        .encode_utf16()
        .flat_map(|u| u.to_ne_bytes())
        .collect();

    let set_status = unsafe {
        RegSetValueExW(
            safe_key.0,
            windows::core::w!("DisplayName"),
            0,
            REG_SZ,
            Some(&display_name_bytes),
        )
    };
    if set_status != ERROR_SUCCESS {
        return Err(windows::core::Error::from(set_status));
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn register_aumid() -> windows::core::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub fn show_toast(title: &str, body: &str) -> windows::core::Result<()> {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::HSTRING;

    let xml = toast_xml(title, body);
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;

    let toast = ToastNotification::CreateToastNotification(&doc)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
    notifier.Show(&toast)?;

    Ok(())
}

#[cfg(not(windows))]
pub fn show_toast(_title: &str, _body: &str) -> windows::core::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toast_xml_structure() {
        let xml = toast_xml("NeoBuds Pro 3: battery low", "Left 14%");
        assert_eq!(
            xml,
            "<toast><visual><binding template=\"ToastGeneric\"><text>NeoBuds Pro 3: battery low</text><text>Left 14%</text></binding></visual></toast>"
        );
    }

    #[test]
    fn test_toast_xml_escaping() {
        let xml = toast_xml("Tom & Jerry <test>", "Quotes: \"hello\" & 'world' > 0");
        assert_eq!(
            xml,
            "<toast><visual><binding template=\"ToastGeneric\"><text>Tom &amp; Jerry &lt;test&gt;</text><text>Quotes: &quot;hello&quot; &amp; &apos;world&apos; &gt; 0</text></binding></visual></toast>"
        );
    }
}
