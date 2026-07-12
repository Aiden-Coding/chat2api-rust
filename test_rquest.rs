use rquest::Client;
fn main() {
    let b = Client::builder().gzip(false).brotli(false).deflate(false);
}
