pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod interface;
pub mod jobs;
pub mod ports;

use std::sync::Arc;

use application::{AppState, ApplicationBackend, DefaultApplicationBackend};
use infrastructure::{
    filesystem::LocalDecisionPacketWriter, provider::ReqwestProviderGateway,
    sqlite::SqliteRepository,
};
use interface::*;
use tauri::Manager;

fn register_handlers(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.invoke_handler(tauri::generate_handler![
        list_workspaces,
        create_workspace,
        open_workspace,
        update_workspace,
        inspect_context,
        create_turn_and_start_run,
        retry_run,
        cancel_run,
        get_run_snapshot,
        update_context_overrides,
        get_route_projection,
        update_view_state,
        compare_runs,
        mark_decision,
        export_decision_packet,
        list_provider_templates,
        list_provider_profiles,
        list_provider_models,
        save_provider_profile,
        list_session_credentials,
        set_session_credential,
        activate_session_credential,
        reorder_session_credentials,
        remove_session_credential,
        test_provider_connection,
    ])
}

pub fn builder(backend: Arc<dyn ApplicationBackend>) -> tauri::Builder<tauri::Wry> {
    register_handlers(tauri::Builder::default().manage(AppState::new(backend)))
}

pub fn run_with_backend(backend: Arc<dyn ApplicationBackend>) {
    run_builder(builder(backend));
}

/// Production entry point. Database migrations and interruption recovery run
/// before the main window can invoke an application command.
pub fn run() {
    let builder = tauri::Builder::default().setup(|app| {
        let data_dir = app.path().app_data_dir()?;
        std::fs::create_dir_all(&data_dir)?;
        let database_path = data_dir.join("thoughsflow.sqlite3");
        let export_root = data_dir.join("exports");
        let backend = tauri::async_runtime::block_on(async move {
            let repository = Arc::new(SqliteRepository::connect(database_path).await?);
            let provider = Arc::new(ReqwestProviderGateway::with_defaults()?);
            let exporter = Arc::new(LocalDecisionPacketWriter::new(export_root));
            let backend = DefaultApplicationBackend::new(
                repository,
                provider.clone(),
                provider.clone(),
                provider,
                exporter,
            );
            backend.initialize().await?;
            Ok::<_, Box<dyn std::error::Error>>(backend)
        })?;
        app.manage(AppState::new(Arc::new(backend)));
        Ok(())
    });
    run_builder(register_handlers(builder));
}

fn run_builder(builder: tauri::Builder<tauri::Wry>) {
    builder
        .run(tauri::generate_context!())
        .expect("failed to run ThoughsFlow desktop application");
}
