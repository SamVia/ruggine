use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;

pub async fn save_data<T: Serialize>(data: &T, path: &str) {
    match serde_json::to_string_pretty(data) {
        Ok(json) => match tokio::fs::write(path, json).await {
            Ok(_) => println!("Dati salvati su disco."),
            Err(err) => eprintln!("Errore nel salvataggio dei dati su disco: {}", err),
        },
        Err(err) => eprintln!("Errore nella serializzazione dei dati: {}", err),
    }
}

pub async fn load_data<T: DeserializeOwned>(path: &str) -> Option<T> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => match serde_json::from_str::<T>(&content) {
            Ok(data) => Some(data),
            Err(err) => {
                eprintln!("Errore nella deserializzazione dei dati: {}", err);
                None
            }
        },
        Err(err) => {
            eprintln!("Errore nel caricamento dei dati da disco (o file non presente): {}", err);
            None
        }
    }
}

// Fonti dei messaggi in chat
#[derive(Serialize, Deserialize,Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChatSource {
    Group(String),
    DirectMessage(String),
    System,
}

// Messaggi scambiati tra client e server
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum MessageType {
    Start(String, String), // (IP, Username) per avviare la connessione
    Login(String, String),      // login con username e password
    CreateGroup(String),        // crea un nuovo gruppo
    InviteToGroup(String, String), // invita un utente a un gruppo
    JoinGroup(String),          // unisciti a un gruppo dopo invito
    LeaveGroup(String),         // lascia un gruppo specifico
    RejectInvite(String),       // rifiuta invito a un gruppo
    MsgGroup(String, String),   // messaggio gruppo
    MsgPrivate(String, String, String), // messaggio dm
    KnownUsers(Vec<String>),    // vettore di nomi utenti noti
    Error(ChatSource, String),
    Info(String),
    Disconnect
}


