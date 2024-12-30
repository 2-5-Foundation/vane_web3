use wasm_builder::WasmBuilder;
use std::path::PathBuf;

fn main() {
    if cfg!(target_arch = "wasm32") {
        WasmBuilder::new()
            .with_current_project()
            .set_file_name("vane_web3_node")  // Set custom output name
            .export_heap_base()
            .import_memory()
            .build()
    }
}