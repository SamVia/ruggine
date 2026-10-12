use tokio::net::TcpListener;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use sysinfo::{CpuExt, System, SystemExt, ProcessExt, Pid};
use std::time::Duration;
use chrono::Local;
use sha2::{Sha256, Digest};
use ruggine::util::{MessageType, ChatSource, save_data, load_data};

// Salvataggio logging e persistenza
#[derive(Serialize, Deserialize, Debug, Clone)]
struct PersistedData {
    // Mappa NomeGruppo -> Lista utenti membri
    groups: HashMap<String, Vec<String>>,
    // Mappa NomeGruppo -> Utenti invitati (pending)
    invites: HashMap<String, Vec<String>>,
    // Mappa Utente -> Lista contatti con cui ha parlato
    #[serde(default)] 
    contacts: HashMap<String, Vec<String>>,
    #[serde(default)]
    users: HashSet<String>,
    // Mappa Username -> Hashed Password
    #[serde(default)] 
    credentials: HashMap<String, String>, 
}

// Stato in memoria del server
struct ServerState {
    // Client connessi ora (non salvati su disco)
    clients: HashMap<String, tokio::sync::mpsc::Sender<String>>,
    // Dati storici (gruppi, inviti, contatti)
    data: PersistedData,
}

// Funzione helper per l'invio dei messaggi
async fn send_message(
    tx: &tokio::sync::mpsc::Sender<String>,
    message: MessageType,
) {
    match serde_json::to_string(&message) {
        Ok(payload) => {
            if let Err(err) = tx.send(payload).await {
                eprintln!("Errore nell'invio del messaggio al client: {}", err);
            }
        }
        Err(err) => eprintln!("Errore nella serializzazione del messaggio: {}", err),
    }
}

// Funzione per hash delle password
fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password);
    hex::encode(hasher.finalize())
}

// --- MAIN SERVER ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Ruggine Server v2.1 (Full Persistence + Dual Logging) ---");
    
    // 1) Carica i dati persistenti
    let initial_data = if let Some(data) = load_data::<PersistedData>("data/server_data.json").await {
        println!("Dati caricati: {} gruppi trovati, {} utenti.", data.groups.len(), data.users.len());
        data
    } else {
        println!("Uso configurazione vuota di default.");
        PersistedData {
            groups: HashMap::new(),
            invites: HashMap::new(),
            contacts: HashMap::new(),
            users: HashSet::new(),
            credentials: HashMap::new(),
        }
    };

    let state = Arc::new(Mutex::new(ServerState {
        clients: HashMap::new(),
        data: initial_data,
    }));

    // Assicura che la cartella logs e data esistano
    if let Err(err) = tokio::fs::create_dir_all("logs").await {
        eprintln!("Errore nella creazione della cartella logs: {}", err);
    }
    if let Err(err) = tokio::fs::create_dir_all("data").await {
        eprintln!("Errore nella creazione della cartella data: {}", err);
    }

    // 2) LOGGER 1: SYSTEM GLOBAL CPU
    tokio::spawn(async move {
        let mut sys = System::new_all();
        loop {
            tokio::time::sleep(Duration::from_secs(120)).await; 
            sys.refresh_cpu();
            let cpu_usage = sys.global_cpu_info().cpu_usage();
            let now = Local::now();
            
            // ASYNC CHANGE: Use tokio::fs::OpenOptions and .await
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open("logs/dump.log")
                .await 
            {
                let log = format!("[{}] SYSTEM | CPU: {:.2}%\n", 
                                                now.format("%Y-%m-%d %H:%M:%S"), 
                                                cpu_usage);
                if let Err(err) = file.write_all(log.as_bytes()).await {
                    eprintln!("Errore nella scrittura del log SYSTEM: {}", err);
                }
            }
        }
    });

    // 3) LOGGER 2: PROCESS SPECIFIC CPU
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
                
                // ASYNC CHANGE: Use tokio::fs::OpenOptions and .await
                if let Ok(mut file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("logs/dump.log")
                    .await 
                {
                    let log = format!(
                        "[{}] SERVER | PID: {} | CPU: {:.2}%\n",
                        now.format("%Y-%m-%d %H:%M:%S"),
                        pid,
                        cpu_usage
                    );
                    if let Err(err) = file.write_all(log.as_bytes()).await {
                        eprintln!("Errore nella scrittura del log SERVER: {}", err);
                    }
                }
            }
        }
    });

    // 4) Setup Networking
    let listener = TcpListener::bind("0.0.0.0:8190").await?;
    println!("Server in ascolto su 0.0.0.0:8190");

    loop {
        // Accetta connessione
        let (socket, addr) = listener.accept().await?;
        println!("Nuova connessione: {}", addr);
        let state_clone = state.clone();

        tokio::spawn(async move {
            let (reader, mut writer) = socket.into_split();
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();
            
            // Canale per inviare messaggi al socket
            let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);

            // Task di scrittura sul socket
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if let Err(err) = writer.write_all(msg.as_bytes()).await {
                        eprintln!("Errore nella scrittura del messaggio sul socket: {}", err);
                        break;
                    }
                    if let Err(err) = writer.write_all(b"\n").await {
                        eprintln!("Errore nella scrittura del terminatore sul socket: {}", err);
                        break;
                    }
                }
            });

            let mut my_username: Option<String> = None;

            // Loop di lettura dal client
            loop {
                line.clear();
                let bytes = buf_reader.read_line(&mut line).await.unwrap_or(0);
                if bytes == 0 { break; } 

                if let Ok(msg) = serde_json::from_str::<MessageType>(line.trim()) {
                    let mut state_lock = state_clone.lock().await;

                    match msg {
                        MessageType::Login(username, password) => {
                            let hashed_input = hash_password(&password);

                            // Controllo se nuovo utente  
                            let is_registered = state_lock.data.credentials.contains_key(&username);

                            if is_registered {
                                // UTENTE ESISTENTE: Controllo Password
                                let stored_hash = state_lock.data.credentials.get(&username).unwrap();
                                if stored_hash != &hashed_input {
                                    // Password errata
                                    send_message(&tx, MessageType::Error(ChatSource::System, "Password errata!".to_string())).await;
                                    return; 
                                }
                            } else {
                                // NUOVO UTENTE: Registrazione automatica al primo accesso
                                state_lock.data.users.insert(username.clone());
                                state_lock.data.credentials.insert(username.clone(), hashed_input);
                                println!("[SAVE] Salvataggio nuova registrazione utente: {}", username);
                                save_data(&state_lock.data, "data/server_data.json").await; 
                            }

                            // Controllo doppia connessione
                            if state_lock.clients.contains_key(&username) {
                                send_message(&tx, MessageType::Error(ChatSource::System, "Username già connesso".to_string())).await;
                            } else {
                                my_username = Some(username.clone());
                                state_lock.clients.insert(username.clone(), tx.clone());
                                println!("Utente Login: {}", username);
                                
                                // Sync Gruppi
                                let mut found_groups = Vec::new();
                                for (group_name, members) in &state_lock.data.groups {
                                    if members.contains(&username) { found_groups.push(group_name.clone()); }
                                }
                                if found_groups.is_empty() {
                                    send_message(&tx, MessageType::Info(format!("Benvenuto {}, sei online!", username))).await;
                                } else {
                                    send_message(&tx, MessageType::Info(format!("Bentornato {}! Ripristino stato...", username))).await;
                                    for g in found_groups { 
                                        send_message(&tx, MessageType::JoinGroup(g)).await; 
                                    }
                                }

                                // Sync Contatti
                                if let Some(contacts) = state_lock.data.contacts.get(&username) {
                                    if !contacts.is_empty() { 
                                        send_message(&tx, MessageType::KnownUsers(contacts.clone())).await; 
                                    }
                                }

                                // Sync Inviti Pendenti
                                for (group_name, invites) in &state_lock.data.invites {
                                    if invites.contains(&username) {
                                        send_message(&tx, MessageType::InviteToGroup(group_name.clone(), "Sistema".to_string()),).await;
                                    }
                                }
                            }
                        },
                        MessageType::CreateGroup(group_name) => {
                            if let Some(user) = &my_username {
                                // Controllo se il nome corrisponde giá ad un gruppo esistente
                                if state_lock.data.groups.contains_key(&group_name) {
                                    send_message(&tx, MessageType::Error(ChatSource::System, "Gruppo esistente".to_string())).await;
                                } else {
                                    state_lock.data.groups.insert(group_name.clone(), vec![user.clone()]);
                                    state_lock.data.invites.insert(group_name.clone(), Vec::new());
                                    println!("[SAVE] Salvataggio creazione gruppo: {}", group_name);
                                    save_data(&state_lock.data, "data/server_data.json").await;
                                    send_message(&tx, MessageType::CreateGroup(group_name)).await;
                                }
                            }
                        },
                        MessageType::InviteToGroup(group_name, target_user) => {
                            if let Some(sender) = &my_username {
                                if let Some(members) = state_lock.data.groups.get(&group_name) {
                                    if members.contains(sender) {
                                        if state_lock.data.invites.get(&group_name).map_or(false, |i| i.contains(&target_user)) ||
                                           state_lock.data.groups.get(&group_name).map_or(false, |m| m.contains(&target_user)) {
                                            send_message(&tx, MessageType::Error(ChatSource::Group(group_name.clone()), "Utente già membro o invitato".to_string())).await;
                                        } else if !state_lock.data.users.contains(&target_user) {
                                            send_message(&tx, MessageType::Error(ChatSource::Group(group_name.clone()), "Utente non registrato".to_string())).await;
                                        } else {
                                            state_lock.data.invites.entry(group_name.clone()).or_default().push(target_user.clone());
                                            println!("[SAVE] Salvataggio invito: utente '{}' nel gruppo '{}'", target_user, group_name);
                                            save_data(&state_lock.data, "data/server_data.json").await;
                                            // Notifica immediata al target se online
                                            if let Some(target_tx) = state_lock.clients.get(&target_user) {
                                                send_message(target_tx, MessageType::InviteToGroup(group_name.clone(), sender.clone()),).await;
                                            }

                                            // Conferma solo al mittente, solo dopo invito riuscito
                                            send_message(&tx, MessageType::Info(format!("Invito inviato con successo a '{}' per il gruppo '{}'.", target_user, group_name))).await;
                                        }
                                    } else { 
                                        send_message(&tx, MessageType::Error(ChatSource::System,"Non sei nel gruppo".to_string())).await; 
                                    }
                                }
                            }
                        },
                        MessageType::JoinGroup(group_name) => {
                            if let Some(user) = &my_username {
                                let is_invited = state_lock.data.invites.get(&group_name).map_or(false, |i| i.contains(user));
                                let is_member = state_lock.data.groups.get(&group_name).map_or(false, |m| m.contains(user));
                                if is_invited || is_member {
                                    if is_invited {
                                        if let Some(inv_list) = state_lock.data.invites.get_mut(&group_name) { inv_list.retain(|x| x != user); }
                                        state_lock.data.groups.entry(group_name.clone()).or_default().push(user.clone());
                                        println!("[SAVE] Salvataggio ingresso nel gruppo: utente '{}' in '{}'", user, group_name);
                                        save_data(&state_lock.data, "data/server_data.json").await;
                                    }
                                    send_message(&tx, MessageType::JoinGroup(group_name)).await;
                                } else { 
                                    send_message(&tx, MessageType::Error(ChatSource::System,"Serve un invito per entrare.".to_string())).await; 
                                }
                            }
                        },
                        MessageType::LeaveGroup(group_name) => {
                            if let Some(user) = &my_username {
                                if let Some(members) = state_lock.data.groups.get_mut(&group_name) {
                                    if members.contains(user) {
                                        // Removing of the user, usage of retain for a safer purpose for possible occuring bug
                                        members.retain(|m| m != user);

                                        if members.is_empty() {
                                            state_lock.data.groups.remove(&group_name);
                                            state_lock.data.invites.remove(&group_name);
                                        }

                                        println!("[SAVE] Salvataggio uscita dal gruppo: utente '{}' da '{}'", user, group_name);
                                        save_data(&state_lock.data, "data/server_data.json").await;
                                        send_message(&tx, MessageType::LeaveGroup(group_name)).await;
                                    } else {
                                        send_message(&tx, MessageType::Error(ChatSource::System, "Non sei membro di questo gruppo.".to_string())).await;
                                    }
                                } else {
                                    send_message(&tx, MessageType::Error(ChatSource::System, format!("Il gruppo '{}' non esiste.", group_name))).await;
                                }
                            }
                        },
                        MessageType::RejectInvite(group_name) => {
                            if let Some(user) = &my_username {
                                if let Some(invites) = state_lock.data.invites.get_mut(&group_name) {
                                    if invites.contains(user) {
                                        invites.retain(|x| x != user);
                                        println!("[SAVE] Salvataggio rifiuto invito: utente '{}' per gruppo '{}'", user, group_name);
                                        save_data(&state_lock.data, "data/server_data.json").await;
                                        send_message(&tx, MessageType::RejectInvite(group_name)).await;
                                    } else { 
                                        send_message(&tx, MessageType::Error(ChatSource::System, "Non hai un invito per questo gruppo.".to_string())).await; 
                                    }
                                } else { 
                                    send_message(&tx, MessageType::Error(ChatSource::System, format!("Il gruppo '{}' non esiste.", group_name))).await; 
                                }
                            }
                        },
                        MessageType::MsgGroup(group_name, text) => {
                            if let Some(sender) = &my_username {
                                if let Some(members) = state_lock.data.groups.get(&group_name) {
                                    if members.contains(sender) {
                                        let msg_struct = MessageType::MsgGroup(group_name.clone(), format!("{}: {}", sender, text));
                                        for member in members {
                                            if let Some(member_tx) = state_lock.clients.get(member) { 
                                                send_message(member_tx, msg_struct.clone()).await; 
                                            }
                                        }
                                    } else { 
                                        send_message(&tx, MessageType::Error(ChatSource::System, "Non sei nel gruppo".to_string())).await; 
                                    }
                                } else { 
                                    send_message(&tx, MessageType::Error(ChatSource::System, format!("Il gruppo '{}' non esiste.", group_name))).await; 
                                }
                            }
                        },
                        MessageType::MsgPrivate(sender_user, target_user, text) => {
                            if !state_lock.data.users.contains(&target_user) {
                                send_message(&tx, MessageType::Error(ChatSource::System, "Utente non registrato".to_string())).await;
                            } else if let Some(sender) = &my_username {
                                // Salvataggio bidirezionale contatto
                                let mut save_needed = false;
                                {
                                    let contacts = state_lock.data.contacts.entry(sender.clone()).or_default();
                                    if !contacts.contains(&target_user) { contacts.push(target_user.clone()); save_needed = true; }
                                }
                                {
                                    let contacts = state_lock.data.contacts.entry(target_user.clone()).or_default();
                                    if !contacts.contains(sender) { contacts.push(sender.clone()); save_needed = true; }
                                }
                                if save_needed {
                                    println!("[SAVE] Salvataggio aggiornamento contatti tra '{}' e '{}'", sender, target_user);
                                    save_data(&state_lock.data, "data/server_data.json").await;
                                }

                                // Invio messaggio
                                if let Some(target_tx) = state_lock.clients.get(&target_user) {
                                    send_message(target_tx, MessageType::MsgPrivate(sender.clone(), target_user.clone(), text.clone())).await;
                                    // Echo al mittente
                                    if let Some(sender_tx) = state_lock.clients.get(&sender_user) {
                                        send_message(sender_tx, MessageType::MsgPrivate(sender.clone(), target_user.clone(), text.clone())).await;
                                    }
                                } else { 
                                    send_message(&tx, MessageType::Error(ChatSource::DirectMessage(target_user.clone()), "Utente offline, ma salvato in rubrica.".to_string())).await; 
                                }
                            }
                        },
                        _ => { 
                            send_message(&tx, MessageType::Error(ChatSource::System, "Tipo messaggio non gestito dal server".to_string())).await; 
                        }
                    }
                }
            }

            // Cleanup Disconnessione
            if let Some(u) = my_username {
                println!("Disconnesso: {}", u);
                let mut state_lock = state_clone.lock().await;
                state_lock.clients.remove(&u);
            }
        });
    }
}