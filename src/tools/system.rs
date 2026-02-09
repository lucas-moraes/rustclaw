use super::Tool;
use serde_json::Value;
use sysinfo::{Disks, System};

pub struct SystemInfoTool;

impl SystemInfoTool {
    pub fn new() -> Self {
        Self
    }

    fn format_bytes(bytes: u64) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
        let mut size = bytes as f64;
        let mut unit_idx = 0;

        while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
            size /= 1024.0;
            unit_idx += 1;
        }

        format!("{:.1} {}", size, UNITS[unit_idx])
    }

    fn format_duration(secs: u64) -> String {
        let days = secs / 86400;
        let hours = (secs % 86400) / 3600;
        let minutes = (secs % 3600) / 60;

        if days > 0 {
            format!("{}d {}h {}m", days, hours, minutes)
        } else if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else {
            format!("{}m", minutes)
        }
    }
}

#[async_trait::async_trait]
impl Tool for SystemInfoTool {
    fn name(&self) -> &str {
        "system_info"
    }

    fn description(&self) -> &str {
        "Retorna informações do sistema (RAM, CPU, disco). Input: {} ou { \"detail\": \"cpu\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let detail = args["detail"].as_str();

        let mut s = System::new_all();
        s.refresh_all();

        
        let total_memory = s.total_memory();
        let used_memory = s.used_memory();
        let free_memory = total_memory - used_memory;
        let memory_percent = if total_memory > 0 {
            (used_memory as f64 / total_memory as f64) * 100.0
        } else {
            0.0
        };

        
        let cpu_count = s.cpus().len();
        let cpu_usage: f32 = s.cpus().iter().map(|cpu| cpu.cpu_usage()).sum::<f32>() / cpu_count as f32;

        
        let hostname = System::host_name().unwrap_or_else(|| "Desconhecido".to_string());
        let os_name = System::name().unwrap_or_else(|| "Desconhecido".to_string());
        let os_version = System::os_version().unwrap_or_else(|| "Desconhecido".to_string());
        let kernel_version = System::kernel_version().unwrap_or_else(|| "Desconhecido".to_string());

        
        let uptime = System::uptime();

        
        let disks = Disks::new_with_refreshed_list();

        match detail {
            Some("cpu") => {
                let cpu_name = s.cpus().first()
                    .map(|cpu| cpu.brand())
                    .unwrap_or("Desconhecido");
                
                Ok(format!(
                    "💻 CPU: {}\n  Cores: {}\n  Uso: {:.1}%\n  Frequência: {:?} MHz",
                    cpu_name,
                    cpu_count,
                    cpu_usage,
                    s.cpus().first().map(|c| c.frequency())
                ))
            }
            Some("memory") | Some("ram") => {
                Ok(format!(
                    "🧠 Memória RAM:\n  Total: {}\n  Usada: {} ({:.1}%)\n  Livre: {}",
                    Self::format_bytes(total_memory),
                    Self::format_bytes(used_memory),
                    memory_percent,
                    Self::format_bytes(free_memory)
                ))
            }
            Some("disk") | Some("disks") => {
                let mut output = String::from("💾 Discos:\n\n");
                for disk in &disks {
                    let total = disk.total_space();
                    let free = disk.available_space();
                    let used = total - free;
                    let percent = if total > 0 {
                        (used as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };

                    output.push_str(&format!(
                        "  {}\n    Total: {}\n    Usado: {} ({:.1}%)\n    Livre: {}\n    Tipo: {}\n\n",
                        disk.mount_point().display(),
                        Self::format_bytes(total),
                        Self::format_bytes(used),
                        percent,
                        Self::format_bytes(free),
                        disk.file_system().to_string_lossy()
                    ));
                }
                Ok(output.trim().to_string())
            }
            _ => {
                
                let mut output = format!(
                    "🖥️  Informações do Sistema\n\n"
                );

                output.push_str(&format!(
                    "📊 Geral:\n  Hostname: {}\n  OS: {} {}\n  Kernel: {}\n  Uptime: {}\n\n",
                    hostname,
                    os_name,
                    os_version,
                    kernel_version,
                    Self::format_duration(uptime)
                ));

                
                output.push_str(&format!(
                    "🧠 Memória:\n  Total: {}\n  Usada: {} ({:.1}%)\n  Livre: {}\n\n",
                    Self::format_bytes(total_memory),
                    Self::format_bytes(used_memory),
                    memory_percent,
                    Self::format_bytes(free_memory)
                ));

                
                output.push_str(&format!(
                    "💻 CPU:\n  Cores: {}\n  Uso médio: {:.1}%\n\n",
                    cpu_count,
                    cpu_usage
                ));

                
                output.push_str("💾 Discos:\n");
                for disk in &disks {
                    let total = disk.total_space();
                    let free = disk.available_space();
                    let used = total - free;
                    let percent = if total > 0 {
                        (used as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };

                    output.push_str(&format!(
                        "  {}: {} / {} ({:.1}% usado)\n",
                        disk.mount_point().display(),
                        Self::format_bytes(used),
                        Self::format_bytes(total),
                        percent,
                    ));
                }

                Ok(output)
            }
        }
    }
}

impl Default for SystemInfoTool {
    fn default() -> Self {
        Self::new()
    }
}
