fn main() {
    dominion_web::platform::install_crash_notice();
    dominion_web::platform::capture_fragment_token();
    dioxus::launch(dominion_web::app::App);
}
