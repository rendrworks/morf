use morf_services::PamAuthenticator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match PamAuthenticator::authenticate(
        "morf-service-that-does-not-exist",
        "morf-invalid-user",
        "morf-invalid-password",
    ) {
        Ok(()) => Err("invalid PAM credentials were accepted".into()),
        Err(error) if error.code().is_some() => {
            println!("PAM rejected invalid credentials: {error}");
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
