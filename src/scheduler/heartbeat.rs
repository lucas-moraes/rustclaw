use sysinfo::System;
use chrono::Local;

pub async fn generate_heartbeat_message() -> String {
    let now = Local::now();
    let mut sys = System::new_all();
    sys.refresh_all();

    let total_mem = sys.total_memory();
    let used_mem = sys.used_memory();
    let mem_percent = if total_mem > 0 {
        (used_mem as f64 / total_mem as f64) * 100.0
    } else {
        0.0
    };

    let uptime_hours = System::uptime() / 3600;

    format!(
        r#"📊 Heartbeat Diário - {}

🧠 Sistema:
   RAM: {} MB / {} MB ({:.1}%)
   Uptime: {}h

Bom dia! Estou aqui e funcionando normalmente. 💚

Use /status para mais detalhes."#,
        now.format("%d/%m/%Y %H:%M"),
        used_mem / 1024,
        total_mem / 1024,
        mem_percent,
        uptime_hours
    )
}

pub async fn generate_system_check_message() -> String {
    let mut sys = System::new_all();
    sys.refresh_all();

    let total_mem = sys.total_memory();
    let used_mem = sys.used_memory();
    let mem_percent = if total_mem > 0 {
        (used_mem as f64 / total_mem as f64) * 100.0
    } else {
        0.0
    };

    let status = if mem_percent > 90.0 {
        "⚠️ ALTO"
    } else if mem_percent > 70.0 {
        "⚡ MÉDIO"
    } else {
        "✅ NORMAL"
    };

    format!(
        r#"🔍 Check do Sistema

Status: {}
RAM: {:.1}% usada

Verificação automática concluída."#,
        status,
        mem_percent
    )
}
