use tauri_winrt_notification::{Duration, Toast};

fn main() {
    let toast = Toast::new(Toast::POWERSHELL_APP_ID);
    let main = std::thread::current();

    toast
        .text1("Hello, World")
        .duration(Duration::Short)
        .add_button("Copy To Clipboard", "copy")
        .add_button("Ignore", "ignore")
        .sound(None)
        .image(
            std::path::Path::new("C:/Users/Admin/Downloads/vault_launcher_ic.png"),
            "The image",
        )
        .on_activated(move |action| {
            println!("user clicked: {}", action.unwrap());
            main.unpark();
            Ok(())
        })
        .on_dismissed(|reason| {
            println!("User cancelled {:?}", reason.unwrap());
            Ok(())
        })
        .show()
        .unwrap();

    std::thread::park();
}
