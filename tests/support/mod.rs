use bb_cli::api::Client;
use bb_cli::credentials::Credentials;
use bb_cli::secret::SecretString;

pub fn client_for(base_url: &str) -> Client {
    let creds = Credentials {
        email: "dev@example.com".into(),
        token: SecretString::from("t0ken-value"),
    };
    Client::new(creds, base_url.to_string()).unwrap()
}
