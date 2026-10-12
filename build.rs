//! Compiles translations, embeds Windows resources, and reads this machine's OAuth identity.

fn main() {
    println!("cargo:rerun-if-changed=.private/spotify-client-id");
    let client_id = std::fs::read_to_string(".private/spotify-client-id").unwrap_or_default();
    let client_id = client_id.trim();
    assert!(
        client_id.is_empty()
            || (client_id.len() == 32 && client_id.bytes().all(|c| c.is_ascii_hexdigit())),
        "Local Spotify Client ID must contain 32 hexadecimal characters"
    );
    // Cargo keeps this generated value under ignored target/. Never print it in application logs.
    println!("cargo:rustc-env=SPOTIFAST_PRIVATE_WEB_CLIENT_ID={client_id}");
    fastframe_i18n::build::compile_catalogs("assets/i18n");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=packaging/windows/spotifast.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("packaging/windows/spotifast.ico")
            .set("ProductName", "Spotifast")
            .set("FileDescription", "A native Spotify client");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=Windows resources not embedded: {error}");
        }
    }
}
