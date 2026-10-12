use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use chrono::Local;
use std::fs;
use std::time::Instant; 
use tokio::fs::OpenOptions;
use sysinfo::{System, SystemExt, ProcessExt, Pid};
use std::time::Duration; 
use ruggine::util::{MessageType, ChatSource};

// Messaggio di chat con contenuto, fonte e colore per la visualizzazione
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub source: ChatSource,
    pub content: String,
    pub color: eframe::egui::Color32,
}

// Struttura per salvare la cronologia locale di gruppi e utenti
#[derive(Serialize, Deserialize, Default)]
struct LocalHistory {
    users: BTreeSet<String>,
    groups: BTreeSet<String>,
}

// Funzione asincrona che gestisce la connessione TCP, login e loop di comunicazione tra GUI e server. Riceve comandi dal thread GUI tramite canale mpsc
async fn run_network(
    mut receiver: tokio::sync::mpsc::Receiver<MessageType>, 
    sender: Sender<MessageType> 
) {
    // FASE 1: ATTESA COMANDO DI AVVIO (IP + USERNAME)
    // Aspetta che la GUI invii il comando di avvio (IP e username) prima di tentare la connessione
    let (stream, initial_username) = loop {
        if let Some(msg) = receiver.recv().await {
            if let MessageType::Start(ip, user) = msg {
                 // Tentativo di connessione all'IP specificato
                 let _ = sender.send(MessageType::Info(format!("Tentativo connessione a {}...", ip)));
                 match TcpStream::connect(format!("{}:8190", ip)).await {
                     Ok(s) => {
                         let _ = sender.send(MessageType::Info("Connesso al server!".to_string()));
                         break (s, user);
                     },
                     Err(e) => {
                         let _ = sender.send(MessageType::Error(ChatSource::System, format!("Impossibile connettersi: {}", e)));
                         // Non esce, aspetta un nuovo tentativo (nuovo click su Entra)
                     }
                 }
            }
        } else {
            return; // Canale chiuso
        }
    };

    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);


    // FASE 2: TASK DI LOGGING (CPU & Username)
    let username_for_log = initial_username.clone();
    tokio::spawn(async move {
        let mut sys = System::new_all();
        let pid = Pid::from(std::process::id() as usize);
        let num_cpus = sys.cpus().len() as f32;
        
        let _ = tokio::fs::create_dir_all("logs").await;
        
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
                        "[{}] GUI | User: {} | PID: {} | CPU: {:.2}%\n",
                        now.format("%Y-%m-%d %H:%M:%S"),
                        username_for_log,
                        pid,
                        cpu_usage
                    );
                    if let Err(err) = file.write_all(log.as_bytes()).await {
                        eprintln!("Errore nella scrittura del log GUI: {}", err);
                    }
                }
            }
        }
    });
    // --- LOGGING LOGIC END ---

    // FASE 3: LOOP DI COMUNICAZIONE
    // Loop principale: gestisce sia i messaggi in arrivo dal server che i comandi dalla GUI
    let mut line = String::new();
    loop {
        tokio::select! {
            Some(msg) = receiver.recv() => {
                // Riceve comandi dalla GUI e li invia al server (eccetto Start)
                // Filtra i messaggi interni che non vanno inviati
                if let MessageType::Start(_, _) = msg { continue; }
                let json = serde_json::to_string(&msg).unwrap();
                let _ = writer.write_all(json.as_bytes()).await;
                let _ = writer.write_all(b"\n").await;
            }
            result = buf_reader.read_line(&mut line) => {
                // Riceve messaggi dal server, li deserializza e li invia al thread GUI
                match result {
                    Ok(0) => {
                        // Server disconnesso (EOF)
                        let _ = sender.send(MessageType::Disconnect);
                        break;
                    },
                    Ok(_) => {
                        if let Ok(msg) = serde_json::from_str::<MessageType>(&line) {
                            let _ = sender.send(msg);
                        }
                        line.clear();
                    }
                    Err(e) => {
                        // Errore nella lettura
                        let _ = sender.send(MessageType::Error(ChatSource::System, format!("Errore di connessione: {}", e)));
                        let _ = sender.send(MessageType::Disconnect);
                        break;
                    }
                }
            }
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
// Modalità di invio: gruppo o privato (DM)
enum SendMode {
    Group,
    Private,
}

// Struttura principale dell'applicazione GUI: gestisce stato, input, cronologia, canali di comunicazione
struct RuggineApp {
    server_ip: String,
    username: String,
    password: String,
    is_logged_in: bool,
    input_text: String,
    disconnect_time: Option<Instant>,
    show_tools: bool,

    // NEW: which conversation is currently selected in the sidebar.
    // None = show all messages (default "All" view).
    selected_chat: Option<ChatSource>,

    // NEW: tracks sources that received a message while NOT selected, for unread badges.
    unread_sources: std::collections::HashSet<ChatSource>,

    cmd_create_group: String,
    cmd_join_group: String,
    cmd_leave_group: String,
    cmd_reject_group: String,
    cmd_invite_group: String,
    cmd_invite_user: String,

    send_mode: SendMode,
    target_name: String,
    
    known_groups: BTreeSet<String>,
    known_users: BTreeSet<String>,
    invites_received: BTreeSet<String>,

    chat_history: Vec<ChatMessage>,
    tx_to_network: tokio::sync::mpsc::Sender<MessageType>,
    rx_from_network: Receiver<MessageType>,
}

impl RuggineApp {
    fn new(
        _cc: &eframe::CreationContext<'_>,
        tx: tokio::sync::mpsc::Sender<MessageType>,
        rx: Receiver<MessageType>
    ) -> Self {
        Self {
            server_ip: "127.0.0.1".to_string(), // Default localhost
            username: String::new(),
            password: String::new(),
            is_logged_in: false,
            input_text: String::new(),
            disconnect_time: None,
            show_tools: false,
            selected_chat: None,
            unread_sources: std::collections::HashSet::new(),
            cmd_create_group: String::new(),
            cmd_join_group: String::new(),
            cmd_leave_group: String::new(),
            cmd_reject_group: String::new(),
            cmd_invite_group: String::new(),
            cmd_invite_user: String::new(),
            send_mode: SendMode::Group,
            target_name: String::new(),
            known_groups: BTreeSet::new(),
            known_users: BTreeSet::new(),
            invites_received: BTreeSet::new(),
            chat_history: Vec::new(),
            tx_to_network: tx,
            rx_from_network: rx,
        }
    }

    // Invia un messaggio al thread di rete
    fn send_msg(&mut self, msg: MessageType) {
        let _ = self.tx_to_network.blocking_send(msg);
    }

    // Carica la cronologia locale (gruppi e utenti) dal file JSON
    fn load_history(&mut self) {
        let filename = format!("data/client_history_{}.json", self.username);
        if let Ok(content) = fs::read_to_string(&filename) {
            if let Ok(data) = serde_json::from_str::<LocalHistory>(&content) {
                self.known_users = data.users;
                self.known_groups = data.groups;
                self.known_users.remove(&self.username);
            }
        }
    }

    // Salva la cronologia locale (gruppi e utenti) su file JSON
    fn save_history(&self) {
        if self.username.is_empty() { return; }
        
        let mut clean_users = self.known_users.clone();
        clean_users.remove(&self.username);

        let data = LocalHistory {
            users: clean_users,
            groups: self.known_groups.clone(),
        };
        let filename = format!("data/client_history_{}.json", self.username);
        if let Ok(json) = serde_json::to_string(&data) {
            let _ = fs::write(filename, json);
        }
    }

    // push a message and mark its source unread if it isn't currently selected.
    fn push_message(&mut self, msg: ChatMessage) {
        let is_selected = match &self.selected_chat {
            None => true, // "All" view — nothing is unread
            Some(sel) => sel == &msg.source,
        };
        if !is_selected {
            self.unread_sources.insert(msg.source.clone());
        }
        self.chat_history.push(msg);
    }

    // returns a filtered view of chat_history for the currently selected conversation.
    fn visible_messages(&self) -> Vec<&ChatMessage> {
        match &self.selected_chat {
            None => self.chat_history.iter().collect(),
            Some(sel) => self.chat_history.iter().filter(|m| &m.source == sel).collect(),
        }
    }
}

// Implementazione della logica GUI: gestisce rendering, input e ricezione messaggi
impl eframe::App for RuggineApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Personalizzazione stile
        let mut style = (*ctx.style()).clone();

        // Arrotonda i bordi di finestre, bottoni e campi di input
        let rounding = egui::Rounding::same(8.0);
        style.visuals.window_rounding = rounding;
        style.visuals.widgets.noninteractive.rounding = rounding;
        style.visuals.widgets.inactive.rounding = rounding;
        style.visuals.widgets.hovered.rounding = rounding;
        style.visuals.widgets.active.rounding = rounding;
        
        // Imposta un Dark Mode con un colore di accento personalizzato
        let mut visuals = egui::Visuals::dark();
        visuals.selection.bg_fill = egui::Color32::from_rgb(88, 101, 242);
        visuals.hyperlink_color = egui::Color32::from_rgb(88, 101, 242);

        style.visuals = visuals;
        ctx.set_style(style);

        // Controlla se è passato abbastanza tempo dal disconnect per chiudere l'app
        if let Some(disconnect_time) = self.disconnect_time {
            if disconnect_time.elapsed().as_secs_f32() >= 5.0 {
                std::process::exit(0);
            }
        }

        // Riceve messaggi dal thread di rete — now uses push_message() to track unread
        while let Ok(msg) = self.rx_from_network.try_recv() {
            let mut changed = false;

            // Gestione di tutti i tipi di messaggi ricevuti dal server
            match msg {
                MessageType::KnownUsers(users) => {
                    let count = users.len();
                    for u in users {
                        if u != self.username {
                            if self.known_users.insert(u) { changed = true; }
                        }
                    }
                    self.push_message(ChatMessage {
                        source: ChatSource::System,
                        content: format!("[RUBRICA] Sincronizzati {} contatti dalla rubrica.", count),
                        color: egui::Color32::GRAY,
                    });
                },
                MessageType::MsgPrivate(sender, target, content) => {
                    if sender != self.username {
                        if self.known_users.insert(sender.clone()) { changed = true; }
                        self.push_message(ChatMessage {
                            source: ChatSource::DirectMessage(sender.clone()),
                            content: format!("{}: {}", sender, content),
                            color: egui::Color32::from_rgb(0, 255, 255),
                        });
                    } else {
                        if self.known_users.insert(target.clone()) { changed = true; }
                        self.push_message(ChatMessage {
                            source: ChatSource::DirectMessage(target.clone()),
                            content: format!("Tu -> {}: {}", target, content),
                            color: egui::Color32::from_rgb(255, 0, 255),
                        });
                    }
                },
                MessageType::MsgGroup(group, content) => {
                    if self.known_groups.insert(group.clone()) { changed = true; }

                    // The server formats group messages as "<sender>: <message>".
                    // Replace sender with "Tu" when the message was sent by this client.
                    let rendered_content = if let Some((sender, text)) = content.split_once(": ") {
                        if sender == self.username {
                            format!("Tu : {}", text)
                        } else {
                            format!("{}: {}", sender, text)
                        }
                    } else {
                        content
                    };

                    self.push_message(ChatMessage {
                        source: ChatSource::Group(group.clone()),
                        content: format!("[{}] {}", group, rendered_content),
                        color: egui::Color32::from_rgb(255, 165, 0),
                    });
                },
                MessageType::JoinGroup(group) => {
                    if self.known_groups.insert(group.clone()) { changed = true; self.invites_received.remove(&group); }
                    self.push_message(ChatMessage {
                        source: ChatSource::Group(group.clone()),
                        content: format!("[INFO] Sei entrato in '{}'", group),
                        color: egui::Color32::GREEN,
                    });
                },
                MessageType::LeaveGroup(group) => {
                    // If the user was viewing that group, reset to "All"
                        if self.selected_chat == Some(ChatSource::Group(group.to_string())) {
                            self.selected_chat = None;
                        }
                    if self.known_groups.remove(&group) { changed = true; }
                    if self.cmd_leave_group == group { self.cmd_leave_group.clear(); }
                    if self.cmd_invite_group == group { self.cmd_invite_group.clear(); }
                    if self.target_name == group && self.send_mode == SendMode::Group {
                        self.target_name.clear();
                    }
                    self.chat_history.retain(|m| m.source != ChatSource::Group(group.to_string()));
                    self.push_message(ChatMessage {
                        source: ChatSource::System,
                        content: format!("[INFO] Hai lasciato il gruppo '{}'", group),
                        color: egui::Color32::GREEN,
                    });
                },
                MessageType::CreateGroup(group) => {
                    if self.known_groups.insert(group.clone()) { changed = true; }
                    self.push_message(ChatMessage {
                        source: ChatSource::Group(group.clone()),
                        content: format!("[INFO] Gruppo '{}' creato", group),
                        color: egui::Color32::GREEN,
                    });
                },
                MessageType::InviteToGroup(group, _) => {
                    if self.invites_received.insert(group.clone()) { changed = true; }
                    self.push_message(ChatMessage {
                        source: ChatSource::Group(group.clone()),
                        content: format!("[INVITO] Sei stato invitato nel gruppo '{}'", group),
                        color : egui::Color32::LIGHT_BLUE,
                    });
                },
                MessageType::RejectInvite(group) => {
                    if self.invites_received.remove(&group) { changed = true; }
                    if self.cmd_reject_group == group { self.cmd_reject_group.clear(); }
                    self.push_message(ChatMessage {
                        source: ChatSource::System,
                        content: format!("[INFO] Hai rifiutato l'invito a '{}'", group),
                        color: egui::Color32::GREEN,
                    });
                },
                MessageType::Info(text) => {
                    self.push_message(ChatMessage {
                        source: ChatSource::System,
                        content: format!("[INFO] {}", text),
                        color: egui::Color32::GREEN,
                    });
                },
                MessageType::Error(source, text) => {
                    self.push_message(ChatMessage {
                        source: source,
                        content: format!("[ERROR] {}", text),
                        color: egui::Color32::RED,
                    });
                    if text.contains("Impossibile connettersi") {
                        self.push_message(ChatMessage {
                            source: ChatSource::System,
                            content: "[SISTEMA] Connessione fallita. App in chiusura tra 5 secondi...".to_string(),
                            color: egui::Color32::RED,
                        });
                        self.is_logged_in = false;
                        self.disconnect_time = Some(Instant::now());
                    }
                },
                MessageType::Disconnect => {
                    self.push_message(ChatMessage {
                        source: ChatSource::System,
                        content: "[SISTEMA] Disconnesso dal server. App in chiusura tra 5 secondi...".to_string(),
                        color: egui::Color32::RED,
                    });
                    self.is_logged_in = false;
                    self.disconnect_time = Some(Instant::now());
                },
                _ => {},
            }

            if changed {
                self.save_history();
            }
        }

        // Gestiamo il layout principale in base allo stato di connessione
        if !self.is_logged_in {
            // Se non siamo loggati, usiamo solo un pannello centrale per la schermata di login
            egui::CentralPanel::default().show(ctx, |ui| {
                // Centriamo tutti gli elementi verticalmente
                ui.vertical_centered(|ui| {
                    ui.add_space(50.0);
                    ui.heading("Ruggine Chat - Login");
                    
                    // Campi di input per IP, Username e Password
                    ui.add_space(20.0);
                    ui.label("IP Server:");
                    ui.text_edit_singleline(&mut self.server_ip);
                    
                    ui.add_space(10.0);
                    ui.label("Scegli il tuo Username:");
                    ui.text_edit_singleline(&mut self.username);
                    
                    ui.add_space(10.0);
                    ui.label("Password:");
                    // .password(true) nasconde i caratteri
                    ui.add(egui::TextEdit::singleline(&mut self.password).password(true)); 

                    ui.add_space(20.0);
                    for msg in &self.chat_history {
                        ui.colored_label(msg.color, &msg.content);
                    }

                    // Pulsante di accesso: scatta la logica solo se tutti e 3 i campi sono compilati
                    if ui.button("Entra nella Chat").clicked() {
                        if !self.username.is_empty() && !self.server_ip.is_empty() && !self.password.is_empty(){
                            self.load_history();
                            // Invia sequenzialmente i comandi TCP per connettersi e autenticarsi
                            self.send_msg(MessageType::Start(self.server_ip.clone(), self.username.clone()));
                            self.send_msg(MessageType::Login(self.username.clone(), self.password.clone()));
                            self.is_logged_in = true;
                        }
                    }
                });
            });
        } else {
            // Se l'interruttore è acceso, mostra il pannello laterale
            if self.show_tools {
                egui::SidePanel::left("left_panel")
                    .resizable(false)
                    .exact_width(220.0)
                    .show(ctx, |ui| {
                        ui.add_space(10.0);
                        ui.heading("🛠 Strumenti");
                        ui.separator();
                        ui.add_space(10.0);

                        ui.label("Crea Gruppo:");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.cmd_create_group).hint_text("Nome Gruppo..."));
                            if ui.button("Crea").clicked() && !self.cmd_create_group.is_empty() {
                                self.send_msg(MessageType::CreateGroup(self.cmd_create_group.clone()));
                                self.cmd_create_group.clear();
                            }
                        });

                        ui.add_space(10.0);
                        ui.label("Entra in Gruppo:");
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_source("join_group_select")
                                .selected_text(if self.cmd_join_group.is_empty() {
                                    if self.invites_received.is_empty() { "Nessun Invito Ricevuto" } else { "Seleziona Gruppo..." }
                                } else { &self.cmd_join_group })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.cmd_join_group, String::new(), "-- Nessuno --");
                                    for group in &self.invites_received {
                                        ui.selectable_value(&mut self.cmd_join_group, group.clone(), group);
                                    }
                                });
                            if ui.add_enabled(!self.invites_received.is_empty(), egui::Button::new("Join")).clicked() {
                                if !self.cmd_join_group.is_empty() {
                                    self.send_msg(MessageType::JoinGroup(self.cmd_join_group.clone()));
                                    self.cmd_join_group.clear();
                                }
                            }
                        });

                        ui.add_space(10.0);
                        ui.label("Lascia Gruppo:");
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_source("leave_group_select")
                                .selected_text(if self.cmd_leave_group.is_empty() {
                                    if self.known_groups.is_empty() { "Nessun Gruppo" } else { "Seleziona Gruppo..." }
                                } else { &self.cmd_leave_group })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.cmd_leave_group, String::new(), "-- Nessuno --");
                                    for group in &self.known_groups {
                                        ui.selectable_value(&mut self.cmd_leave_group, group.clone(), group);
                                    }
                                });
                            if ui.add_enabled(!self.known_groups.is_empty(), egui::Button::new("Leave")).clicked() {
                                if !self.cmd_leave_group.is_empty() {
                                    self.send_msg(MessageType::LeaveGroup(self.cmd_leave_group.clone()));
                                    self.cmd_leave_group.clear();
                                }
                            }
                        });

                        ui.add_space(10.0);
                        ui.label("Rifiuta Invito:");
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_source("reject_group_select")
                                .selected_text(if self.cmd_reject_group.is_empty() {
                                    if self.invites_received.is_empty() { "Nessun Invito Ricevuto" } else { "Seleziona Gruppo..." }
                                } else { &self.cmd_reject_group })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.cmd_reject_group, String::new(), "-- Nessuno --");
                                    for group in &self.invites_received {
                                        ui.selectable_value(&mut self.cmd_reject_group, group.clone(), group);
                                    }
                                });
                            if ui.add_enabled(!self.invites_received.is_empty(), egui::Button::new("Reject")).clicked() {
                                if !self.cmd_reject_group.is_empty() {
                                    self.send_msg(MessageType::RejectInvite(self.cmd_reject_group.clone()));
                                }
                            }
                        });

                        ui.add_space(10.0);
                        ui.label("Invita Utente:");
                        egui::ComboBox::from_id_source("invite_group_select")
                            .selected_text(if self.cmd_invite_group.is_empty() { "Seleziona Gruppo..." } else { &self.cmd_invite_group })
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.cmd_invite_group, String::new(), "-- Nessuno --");
                                ui.separator(); 
                                for group in &self.known_groups {
                                    ui.selectable_value(&mut self.cmd_invite_group, group.clone(), group);
                                }
                            });
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.cmd_invite_user).hint_text("Utente..."));
                            if ui.add_enabled(
                                !self.cmd_invite_group.is_empty() && !self.cmd_invite_user.is_empty(),
                                egui::Button::new("Invia Invito")
                            ).clicked() {
                                self.send_msg(MessageType::InviteToGroup(self.cmd_invite_group.clone(), self.cmd_invite_user.clone()));
                                self.cmd_invite_user.clear();
                            }
                        });
                    });
            }

            // Sidebar conversazioni
            egui::SidePanel::left("conversations_panel")
                .resizable(false)
                .exact_width(160.0)
                .show(ctx, |ui| {
                    ui.add_space(8.0);
                    ui.heading("💬 Chat");
                    ui.separator();
                    ui.add_space(4.0);

                    // ── ALL ──
                    let all_selected = self.selected_chat.is_none();
                    let all_label = egui::RichText::new("📋 Tutte")
                        .color(if all_selected { egui::Color32::WHITE } else { egui::Color32::GRAY });
                    if ui.add(egui::SelectableLabel::new(all_selected, all_label)).clicked() {
                        self.selected_chat = None;
                    }

                    ui.add_space(6.0);

                    // ── SYSTEM ──
                    let sys_src = ChatSource::System;
                    let sys_selected = self.selected_chat.as_ref() == Some(&sys_src);
                    let sys_unread = self.unread_sources.contains(&sys_src);
                    let sys_text = if sys_unread { "🔔 Sistema *" } else { "🔔 Sistema" };
                    let sys_label = egui::RichText::new(sys_text)
                        .color(if sys_selected { egui::Color32::WHITE } else { egui::Color32::GRAY });
                    if ui.add(egui::SelectableLabel::new(sys_selected, sys_label)).clicked() {
                        self.selected_chat = Some(sys_src.clone());
                        self.unread_sources.remove(&sys_src);
                    }

                    // ── GROUPS ──
                    if !self.known_groups.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("GRUPPI").small().weak());
                        ui.separator();

                        let groups: Vec<String> = self.known_groups.iter().cloned().collect();
                        for group in groups {
                            let src = ChatSource::Group(group.clone());
                            let is_selected = self.selected_chat.as_ref() == Some(&src);
                            let has_unread = self.unread_sources.contains(&src);
                            let invite_pending = self.invites_received.contains(&group);

                            // Build label: truncate long names, append unread dot or invite badge
                            let display_name: String = if group.len() > 14 {
                                format!("{}…", &group[..13])
                            } else {
                                group.clone()
                            };
                            let badge = if invite_pending { " ✉" } else if has_unread { " *" } else { "" };
                            let row_text = format!("# {}{}", display_name, badge);

                            let label_color = if is_selected {
                                egui::Color32::WHITE
                            } else if has_unread || invite_pending {
                                egui::Color32::from_rgb(255, 165, 0)
                            } else {
                                egui::Color32::GRAY
                            };

                            let rich = egui::RichText::new(row_text).color(label_color);
                            if ui.add(egui::SelectableLabel::new(is_selected, rich)).clicked() {
                                self.selected_chat = Some(src.clone());
                                self.unread_sources.remove(&src);
                            }
                        }
                    }

                    // DMs
                    if !self.known_users.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("MESSAGGI DIRETTI").small().weak());
                        ui.separator();

                        let users: Vec<String> = self.known_users.iter().cloned().collect();
                        for user in users {
                            let src = ChatSource::DirectMessage(user.clone());
                            let is_selected = self.selected_chat.as_ref() == Some(&src);
                            let has_unread = self.unread_sources.contains(&src);

                            let display_name: String = if user.len() > 14 {
                                format!("{}…", &user[..13])
                            } else {
                                user.clone()
                            };
                            let badge = if has_unread { " *" } else { "" };
                            let row_text = format!("@ {}{}", display_name, badge);

                            let label_color = if is_selected {
                                egui::Color32::WHITE
                            } else if has_unread {
                                egui::Color32::LIGHT_BLUE
                            } else {
                                egui::Color32::GRAY
                            };

                            let rich = egui::RichText::new(row_text).color(label_color);
                            if ui.add(egui::SelectableLabel::new(is_selected, rich)).clicked() {
                                self.selected_chat = Some(src.clone());
                                self.unread_sources.remove(&src);
                            }
                        }
                    }
                });

            // Pannello centrale
            egui::CentralPanel::default().show(ctx, |ui| {

                // Header
                ui.horizontal(|ui| {
                    if ui.button("☰").clicked() {
                        self.show_tools = !self.show_tools;
                    }
                    ui.add_space(10.0);

                    // Show the active conversation name in the header
                    let chat_title = match &self.selected_chat {
                        None => format!("👤 {} — Tutte le chat", self.username),
                        Some(ChatSource::System) => format!("👤 {} — Sistema", self.username),
                        Some(ChatSource::Group(g)) => format!("👤 {} — #{}", self.username, g),
                        Some(ChatSource::DirectMessage(u)) => format!("👤 {} — @{}", self.username, u),
                    };
                    ui.heading(chat_title);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("{}", Local::now().format("%H:%M")));
                    });
                });
                ui.separator();

                // Message area — filtered by selected_chat
                let height = ui.available_height() - 80.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .max_height(height)
                    .show(ui, |ui| {
                        ui.set_min_height(height);
                        // Collect first to avoid borrow conflict
                        let visible: Vec<(String, egui::Color32)> = self.visible_messages()
                            .iter()
                            .map(|m| (m.content.clone(), m.color))
                            .collect();
                        for (content, color) in visible {
                            ui.colored_label(color, &content);
                        }
                    });

                ui.separator();

                // Send bar — destination selector
                ui.horizontal(|ui| {
                    let previous_mode = self.send_mode;
                    egui::ComboBox::from_id_source("mode_selector")
                        .selected_text(match self.send_mode {
                            SendMode::Group => "Gruppo",
                            SendMode::Private => "Privato (DM)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.send_mode, SendMode::Group, "Gruppo");
                            ui.selectable_value(&mut self.send_mode, SendMode::Private, "Privato (DM)");
                        });

                    if previous_mode != self.send_mode {
                        self.target_name.clear();
                    }

                    let combo_id = "recipient_selector";
                    let list_to_show = match self.send_mode {
                        SendMode::Group => &self.known_groups,
                        SendMode::Private => &self.known_users,
                    };
                    let count = list_to_show.len();
                    ui.label(egui::RichText::new(format!("({} salvati)", count)).weak().small());

                    egui::ComboBox::from_id_source(combo_id)
                        .selected_text(&self.target_name)
                        .show_ui(ui, |ui| {
                            if list_to_show.is_empty() {
                                ui.label("Nessun contatto recente");
                            }
                            for item in list_to_show {
                                ui.selectable_value(&mut self.target_name, item.clone(), item);
                            }
                        });
                    if self.send_mode == SendMode::Private {
                        ui.add(egui::TextEdit::singleline(&mut self.target_name).hint_text("Nome Destinatario..."));
                    }
                });

                // Send bar — text input + send button
                ui.horizontal(|ui| {
                    let response = ui.add_sized(
                        [ui.available_width() - 60.0, 30.0],
                        egui::TextEdit::singleline(&mut self.input_text)
                            .hint_text("Scrivi un messaggio... (Premi Invio)")
                    );

                    let send_clicked = ui.button("Invia >>").clicked();
                    let enter_pressed = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    if (send_clicked || enter_pressed) && !self.input_text.is_empty() {
                        if !self.target_name.is_empty() {
                            let mut changed = false;
                            match self.send_mode {
                                SendMode::Group => {
                                    self.send_msg(MessageType::MsgGroup(self.target_name.clone(), self.input_text.clone()));
                                    if self.known_groups.insert(self.target_name.clone()) { changed = true; }
                                },
                                SendMode::Private => {
                                    self.send_msg(MessageType::MsgPrivate(self.username.clone(), self.target_name.clone(), self.input_text.clone()));
                                    changed = true;
                                }
                            }
                            if changed { self.save_history(); }
                            self.input_text.clear();
                            response.request_focus();
                        } else {
                            self.push_message(ChatMessage {
                                source: ChatSource::System,
                                content: "[UI] Seleziona un destinatario prima di inviare!".to_string(),
                                color: egui::Color32::YELLOW,
                            });
                        }
                    }
                });
            });
        }

        ctx.request_repaint();
    }
}

// Entry point: avvia thread di rete e lancia la GUI
fn main() -> eframe::Result<()> {
    let (tx_gui, rx_net) = tokio::sync::mpsc::channel(100);
    let (tx_net, rx_gui) = channel();

    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            run_network(rx_net, tx_net).await;
        });
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([860.0, 600.0]), // widened slightly to fit the new sidebar
        ..Default::default()
    };

    eframe::run_native(
        "Ruggine Chat",
        options,
        Box::new(|cc| Box::new(RuggineApp::new(cc, tx_gui, rx_gui))),
    )
}