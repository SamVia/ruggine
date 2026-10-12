use tokio::net::TcpStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::fs::OpenOptions;
use ruggine::util::MessageType;
use std::io::{self, Write};
use sysinfo::{System, SystemExt, ProcessExt, Pid}; // Imports for logging
use std::time::Duration;
use chrono::Local;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Ruggine Client CLI ---");
    
    // 1. INPUT IP SERVER
    print!("Inserisci IP Server (es. 192.168.1.5): ");
    io::stdout().flush()?;
    let mut server_ip = String::new();
    io::stdin().read_line(&mut server_ip)?;
    let server_ip = server_ip.trim();

    // Costruzione dell'endpoint target: hostname:porta
    let target = if server_ip.is_empty() { "127.0.0.1:8190".to_string() } else { format!("{}:8190", server_ip) };
    println!("Connessione a {}", target);
    
    // Apertura della connessione TCP al server
    let stream = TcpStream::connect(&target).await?;
    let (reader, mut writer) = stream.into_split();
    
    // 2. INPUT USERNAME
    print!("Inserisci il tuo Username: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username)?;
    let username = username.trim().to_string();

     // 3. INPUT PASSWORD
    print!("Inserisci la tua Password: ");
    io::stdout().flush()?;
    let mut password = String::new();
    io::stdin().read_line(&mut password)?;
    let password = password.trim().to_string();

    // --- LOGGING LOGIC START ---
    let username_for_log = username.clone();
    tokio::spawn(async move {
        let mut sys = System::new_all();
        let pid = Pid::from(std::process::id() as usize);
        let num_cpus = sys.cpus().len() as f32; 
        
        loop {
            tokio::time::sleep(Duration::from_secs(120)).await;
            sys.refresh_processes();
            
            if let Some(process) = sys.process(pid) {
                let cpu_usage = process.cpu_usage() / num_cpus;
                let now = Local::now();
                
                // ASYNC CHANGE
                if let Ok(mut file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("logs/dump.log")
                    .await 
                {
                    let log = format!(
                        "[{}] CLI | User: {} | PID: {} | CPU: {:.2}%\n",
                        now.format("%Y-%m-%d %H:%M:%S"),
                        username_for_log,
                        pid,
                        cpu_usage
                    );
                    if let Err(err) = file.write_all(log.as_bytes()).await {
                        eprintln!("Errore nella scrittura del log CLI: {}", err);
                    }
                }
            }
        }
    });

    // --- LOGGING LOGIC END ---

    println!("\nComandi disponibili:");
    println!("  /create <gruppo>                Crea un nuovo gruppo");
    println!("  /invite <gruppo> <utente>       Invita un utente nel gruppo");
    println!("  /join <gruppo>                  Accetta invito e entra nel gruppo");
    println!("  /leave <gruppo>                 Esci da un gruppo");
    println!("  /reject <gruppo>                Rifiuta invito al gruppo");
    println!("  /msg <gruppo> <messaggio...>    Invia messaggio al gruppo");
    println!("  /dm <utente> <messaggio...>     Invia messaggio privato");
    println!("  exit                            Chiude il client\n");

    // Serializza il messaggio di login in JSON e lo invia al server
    let login_msg = serde_json::to_string(&MessageType::Login(username.clone(), password))?;
    writer.write_all(login_msg.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    
    let username_clone = username.clone(); // Clone for the reading thread

    // Task Lettura: legge dal socket in modo asincrono, decodifica i JSON e stampa a schermo i messaggi
    tokio::spawn(async move {
        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            let bytes = buf_reader.read_line(&mut line).await.unwrap_or(0);
            if bytes == 0 { 
                println!("Server disconnesso.");
                std::process::exit(0);
            }
            
            if let Ok(msg) = serde_json::from_str::<MessageType>(&line) {
                match msg {
                    MessageType::Info(text) => println!("\x1b[32m[INFO]\x1b[0m {}", text),
                    MessageType::Error(_, err) => println!("\x1b[31m[ERRORE]\x1b[0m {}", err),
                    MessageType::CreateGroup(group) => {
                        println!("\x1b[32m[INFO]\x1b[0m Gruppo '{}' creato con successo", group);
                    }
                    MessageType::InviteToGroup(group, _) => {
                        println!(
                            "\x1b[34m[INVITO]\x1b[0m Sei stato invitato nel gruppo '{}'",
                            group
                        );
                    }
                    MessageType::JoinGroup(group) => {
                        println!("\x1b[32m[INFO]\x1b[0m Sei entrato in '{}'", group);
                    }
                    MessageType::LeaveGroup(group) => {
                        println!("\x1b[32m[INFO]\x1b[0m Hai lasciato il gruppo '{}'", group);
                    }
                    MessageType::RejectInvite(group) => {
                        println!("\x1b[32m[INFO]\x1b[0m Hai rifiutato l'invito a '{}'", group);
                    }
                    MessageType::MsgPrivate(sender, target, text) => {
                        if sender == username_clone.as_str() {
                            println!("\x1b[35m[DM] Tu -> {}: {}\x1b[0m", target, text);
                        } else {
                            println!("\x1b[36m[DM] {}: {}\x1b[0m", sender, text);
                        }
                    },
                    MessageType::MsgGroup(group, text) => {
                        // The server formats group messages as "<sender>: <message>".
                        // Replace sender with "Tu" when the message was sent by this client.
                        if let Some((sender, content)) = text.split_once(": ") {
                            if sender == username_clone.as_str() {
                                println!("\x1b[33m[{}] Tu: {}\x1b[0m", group, content);
                            } else {
                                println!("\x1b[33m[{}] {}: {}\x1b[0m", group, sender, content);
                            }
                        } else {
                            // Fallback for unexpected message format.
                            println!("\x1b[33m[{}] {}\x1b[0m", group, text);
                        }
                    },
                    MessageType::KnownUsers(users) => println!("\x1b[90m[RUBRICA] Contatti noti: {:?}\x1b[0m", users),
                    _ => {
                        println!("\x1b[90m[DEBUG]\x1b[0m Messaggio non gestito ricevuto");
                    }
                }
                print!("> ");
                io::stdout().flush().unwrap();
            }
        }
    });

    // Loop Scrittura: leggere input da stdin, tradurre in `MessageType` e inviare al server
    let stdin = io::stdin();
    let mut input = String::new();

    loop {
        print!("> ");
        io::stdout().flush()?;
        
        input.clear();
        stdin.read_line(&mut input)?;
        let parts: Vec<&str> = input.trim().split_whitespace().collect();

        if parts.is_empty() { continue; }

        // Parsing comandi
        let msg_opt = match parts[0] {
            "/create" if parts.len() > 1 => Some(MessageType::CreateGroup(parts[1].to_string())),
            "/invite" if parts.len() > 2 => Some(MessageType::InviteToGroup(parts[1].to_string(), parts[2].to_string())),
            "/join" if parts.len() > 1 => Some(MessageType::JoinGroup(parts[1].to_string())),
            "/leave" if parts.len() > 1 => Some(MessageType::LeaveGroup(parts[1].to_string())),
            "/reject" if parts.len() > 1 => Some(MessageType::RejectInvite(parts[1].to_string())),
            "/msg" if parts.len() > 2 => {
                let content = parts[2..].join(" ");
                Some(MessageType::MsgGroup(parts[1].to_string(), content))
            },
            "/dm" if parts.len() > 2 => {
                let content = parts[2..].join(" ");
                Some(MessageType::MsgPrivate(username.to_string(), parts[1].to_string(), content))
            },
            "exit" => break,
            _ => {
                println!("Comando non riconosciuto.");
                None
            }
        };

        if let Some(msg) = msg_opt {
            match serde_json::to_string(&msg) {
                Ok(json) => {
                    if let Err(err) = writer.write_all(json.as_bytes()).await {
                        eprintln!("Errore nell'invio del messaggio al server: {}", err);
                        continue;
                    }
                    if let Err(err) = writer.write_all(b"\n").await {
                        eprintln!("Errore nell'invio del terminatore di messaggio al server: {}", err);
                        continue;
                    }
                }
                Err(err) => {
                    eprintln!("Errore nella serializzazione del comando: {}", err);
                    continue;
                }
            }
        }
    }

    Ok(())
}