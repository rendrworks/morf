use mold_services::PamAuthenticator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match PamAuthenticator::authenticate(
        "mold-service-that-does-not-exist",
        "mold-invalid-user",
        "mold-invalid-password",
    ) {
        Ok(()) => Err("invalid PAM credentials were accepted".into()),
        Err(error) if error.code().is_some() => {
            println!("PAM rejected invalid credentials: {error}");
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
