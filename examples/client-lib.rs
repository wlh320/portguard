use portguard::client;

#[no_mangle]
extern "C" fn portguard_run_client(port: u16) {
    env_logger::init();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { client::Client::run_client(port, None).await })
        .unwrap();
}