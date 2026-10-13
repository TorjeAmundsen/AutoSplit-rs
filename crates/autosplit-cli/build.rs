fn main() {
    slint_build::compile("ui/main.slint").expect("the UI compiles");
    // The icon Explorer shows for AutoSplit.exe and AutoSplit.com. Does nothing outside Windows.
    embed_resource::compile("icon.rc", embed_resource::NONE).manifest_optional().expect("the icon compiles");
}
