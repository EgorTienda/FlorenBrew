use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::{Read, Write};
use std::thread;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "florenbrew")]
#[command(about = "A lightweight GitHub release installer & manager", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Установить утилиту/релиз из GitHub репозитория (например: owner/repo)
    Install {
        /// Репозиторий на GitHub в формате author/repo
        repo: String,
    },
    /// Удалить установленный пакет
    Remove {
        /// Имя пакета
        name: String,
    },
    /// Показать список установленных через florenbrew утилит
    List,
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Install { repo } => {
            println!("[*] Запрос релиза для репозитория: {}", repo);
            if let Err(e) = install_github_release(repo) {
                eprintln!("[!] Ошибка установки: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Remove { name } => {
            println!("[*] Удаление пакета: {}", name);
            // Логика удаления бинарника
        }
        Commands::List => {
            println!("[*] Список установленных пакетов florenbrew:");
            // Логика сканирования директории установки (~/.local/bin)
        }
    }
}

/// Функция получения последнего релиза с GitHub API и скачивания бинарника
fn install_github_release(repo: &str) -> Result<(), Box<dyn std::error::Error>> {
    let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    
    // Защита от rate-limit / вежливый клиент к GitHub API
    println!("[*] Ожидание перед запросом к GitHub API...");
    thread::sleep(Duration::from_secs(1));

    println!("[*] Подключение к GitHub API...");
    
    let resp: serde_json::Value = ureq::get(&api_url)
        .set("User-Agent", "florenbrew-rust-client")
        .call()?
        .into_json()?;

    let tag_name = resp["tag_name"].as_str().unwrap_or("unknown");
    println!("[+] Найден последний релиз: {}", tag_name);

    // Автоматически определяем ОС и архитектуру текущей системы
    let (os_keyword, arch_keyword) = get_target_filters();
    println!("[*] Определена система: ОС = '{}', Архитектура = '{}'", os_keyword, arch_keyword);

    // Ищем подходящий актив в релизе
    if let Some(assets) = resp["assets"].as_array() {
        for asset in assets {
            let name = asset["name"].as_str().unwrap_or("").to_lowercase();
            let download_url = asset["browser_download_url"].as_str().unwrap_or("");
            
            if name.contains(&os_keyword) && name.contains(&arch_keyword) {
                if arch_keyword == "amd64" && name.contains("arm64") {
                    continue;
                }

                let original_name = asset["name"].as_str().unwrap_or("binary");
                println!("[*] Найден подходящий файл: {}", original_name);
                println!("[*] Ссылка для скачивания: {}", download_url);
                
                download_and_install(download_url, original_name)?;
                return Ok(());
            }
        }
    }

    Err(format!(
        "Не удалось найти подходящий файл для ОС '{}' и архитектуры '{}' в этом релизе.",
        os_keyword, arch_keyword
    ).into())
}

/// Определяет ключевые слова для текущей ОС и архитектуры
fn get_target_filters() -> (String, String) {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let os_keyword = match os {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        _ => os,
    };

    let arch_keyword = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        _ => arch,
    };

    (os_keyword.to_string(), arch_keyword.to_string())
}

/// Загрузка файла с прогресс-баром (indicatif) и распаковка архивов (tar / flate2)
fn download_and_install(url: &str, filename: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("[*] Скачивание файла...");
    let response = ureq::get(url).call()?;
    
    // Получаем размер файла для прогресс-бара
    let total_size = response
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    // Инициализируем прогресс-бар (indicatif)
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let install_dir = dirs::home_dir()
        .ok_or("Не удалось определить домашнюю директорию")?
        .join(".local/bin");
    
    fs::create_dir_all(&install_dir)?;
    
    let dest_path = install_dir.join(filename);
    let mut dest_file = fs::File::create(&dest_path)?;

    // Читаем поток данных порциями и обновляем прогресс-бар
    let mut reader = response.into_reader();
    let mut buffer = [0; 8192];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        dest_file.write_all(&buffer[..n])?;
        pb.inc(n as u64);
    }
    pb.finish_with_message("Скачивание завершено");

    // Если скачанный файл является архивом .tar.gz / .tgz, распаковываем его (используем tar и flate2)
    if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
        println!("[*] Распаковка архива .tar.gz...");
        let tar_file = fs::File::open(&dest_path)?;
        let decompressed = flate2::read::GzDecoder::new(tar_file);
        let mut archive = tar::Archive::new(decompressed);
        archive.unpack(&install_dir)?;
        println!("[+] Архив успешно распакован в {:?}", install_dir);
    } else {
        // Делаем бинарный файл исполняемым (chmod +x) на Linux/Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(&dest_path)?.permissions();
            perm.set_mode(0o755);
            fs::set_permissions(&dest_path, perm)?;
        }
    }

    println!("[+] Успешно установлено в: {:?}", dest_path);
    Ok(())
}
