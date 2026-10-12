# Manuale del Progettista - Ruggine

## 1. Architettura di Sistema

Ruggine è un'applicazione basata sull'architettura **Client-Server**. I vari protocolli che interagiscono tra di loro utilizzano il runtime asincrono **Tokio**.

### Tecnologie Fondamentali

* **Runtime Asincrono:** `tokio` gestione I/O non bloccante, TCP sockets, multi-threading.
* **Comunicazione Thread-Safe:** `tokio::sync::mpsc` e `std::sync::mpsc` per il passaggio di messaggi tra task e thread.
* **Serializzazione:** `serde` e `serde_json` per la conversione delle strutture dati in JSON.
* **Interfaccia Grafica:** `egui` e `eframe` Per la creazione della GUI.
* **Sicurezza:** `sha2` hashing SHA-256 per le password.
* **Telemetria:** `sysinfo` monitoraggio risorse di sistema.

---

## 2. Struttura del Codice Sorgente

Il progetto utilizza Cargo workspaces/binaries per gestire più eseguibili all'interno dello stesso pacchetto:

* **`src/util.rs`**: Libreria condivisa. Contiene i tipi di dato comuni (`MessageType`, `ChatSource`) e le funzioni di utilità per il salvataggio/caricamento asincrono dei file JSON (`save_data`, `load_data`).
* **`src/bin/server.rs`**: L'eseguibile del backend. Gestisce le connessioni TCP in ingresso, lo stato centralizzato (Mutex) e il salvataggio persistente.
* **`src/bin/client.rs`**: L'eseguibile del client CLI. Crea due task paralleli: uno per leggere da `stdin` e inviare al server, l'altro per leggere dal socket TCP e stampare su `stdout`.
* **`src/bin/gui_client.rs`**: L'eseguibile del client grafico. Utilizza thread sincrono per la UI `egui` e thread asincrono `tokio` per la rete.

---

## 3. Il Protocollo di Rete (`MessageType`)

La comunicazione avviene tramite socket TCP bidirezionali. Il protocollo è testuale e delimitato da newline (`\n`). Ogni riga rappresenta una variante dell'enum `MessageType` definito in `util.rs`.

```rust
pub enum MessageType {
    Start(String, String),              // (IP, Username) - Interno alla GUI
    Login(String, String),              // (Username, Password) - Autenticazione 
    CreateGroup(String),                // (NomeGruppo) - Richiesta di creazione gruppo
    InviteToGroup(String, String),      // (NomeGruppo, TargetUser) - Invito ad un gruppo
    JoinGroup(String),                  // (NomeGruppo) - Accetta invito
    LeaveGroup(String),                 // (NomeGruppo) - Esci dal gruppo
    RejectInvite(String),               // (NomeGruppo) - Rifiuta invito
    MsgGroup(String, String),           // (NomeGruppo, Testo) - Messaggio di gruppo
    MsgPrivate(String, String, String), // (Mittente, Destinatario, Testo) - Messaggio privato
    KnownUsers(Vec<String>),            // Sincronizzazione rubrica dal server
    Error(ChatSource, String),          // Messaggio di errore strutturato
    Info(String),                       // Messaggi informativi di sistema
    Disconnect                          // Segnale di chiusura connessione
}

```

**Nota di Design:** L'enum `MessageType` agisce sia da *Comando* (dal client al server) sia da *Evento/Aggiornamento* (dal server al client).

---

## 4. Concorrenza e Gestione dello Stato

### Lato Server

Il server deve gestire molteplici connessioni e aggiornare uno stato globale.

* **Stato Globale (`ServerState`)**: È racchiuso in un `Arc<Mutex<ServerState>>`. L'`Arc` (Atomically Reference Counted) permette di condividere il puntatore tra i vari task di Tokio, mentre il `tokio::sync::Mutex` assicura che solo un task alla volta possa modificare i dati (es. aggiungere un utente a un gruppo).
* **Pattern MPSC per l'invio**: Ogni connessione client in ingresso viene divisa (`socket.into_split()`). La lettura avviene in un loop, ma la scrittura è delegata a un task separato. Il server comunica con il task di scrittura del client tramite un canale `mpsc::Sender<String>`, salvato nell'HashMap `clients`.

### Lato Client CLI

Il client da riga di comando opera interamente all'interno del runtime asincrono fornito dalla macro `#[tokio::main]`. Una volta stabilita la connessione al server, il socket TCP viene suddiviso in due meta' indipendenti (`reader` e `writer`) tramite `stream.into_split()`.

* **Task asincrono di lettura** (background RX): l'applicazione avvia un task concorrente tramite `tokio::spawn`, dedicato esclusivamente all'ascolto dei messaggi.
  * Utilizza `BufReader` per leggere lo stream di rete riga per riga in modo non bloccante tramite `read_line(&mut line).await`.
  * Quando riceve un payload JSON, effettua la deserializzazione nell'enum `MessageType` e stampa l'output nel terminale.
* **Loop principale di input** (TX sincrono): il thread principale esegue un ciclo continuo per acquisire l'input da tastiera tramite `std::io::stdin().read_line()`.
  * L'input testuale viene suddiviso e analizzato in base al prefisso per riconoscere i comandi utente (es. `/create`, `/join`, `/msg`, `/dm`).
  * Una volta identificato il comando, i parametri vengono mappati nel corrispettivo `MessageType`, serializzati in JSON e scritti nello stream tramite il writer asincrono.
* **Nota architetturale**:
  * `std::io::stdin().read_line()` nel loop principale e' un'operazione bloccante a livello OS.
  * Il task di lettura rete e' separato con `tokio::spawn`, quindi continua a girare su un worker thread del runtime.
  * Risultato: i messaggi in arrivo dal server restano in tempo reale anche mentre l'utente sta digitando.

### Lato Client GUI

Il framework `egui` è sincrono e blocca il thread principale, incompatibile con il runtime asincrono di Tokio.

* **Pattern Multi-Thread**: L'app risolve questo problema lanciando un thread di sistema nativo (`std::thread::spawn`) che inizializza un runtime Tokio dedicato (`tokio::runtime::Runtime::new()`).
* **Comunicazione Inter-Thread**:
* `tx_to_network` / `rx_from_network`: Canali per passare dati dalla UI (sincrona) al thread di rete (asincrono) e viceversa.



---

## 5. Telemetria e Logging in Background

Tutti e tre i file eseguibili implementano un sistema di logging passivo usando il crate `sysinfo`.

* Un task asincrono con un timer (`tokio::time::sleep`) viene avviato all'avvio dell'app.
* Risvegliandosi ogni intervallo (es. 120 secondi), il task usa `sysinfo` per catturare l'uso della CPU (`cpu_usage / num_cpus`).
* Il dato viene appeso asincronamente tramite `tokio::fs::OpenOptions` nel file condiviso `logs/dump.log`.

---

## 6. Guida Estensione: Aggiungere una Nuova Funzionalità

Per aggiungere un nuovo comando (es. `/ping` per testare la latenza):

1. **Modifica il Protocollo (`util.rs`)**:
Aggiungi la variante `Ping` all'enum `MessageType`.
```rust
pub enum MessageType {
    // ... altre varianti
    Ping,
    Pong,
}

```


2. **Gestisci l'Input nel Client (`client.rs` / `gui_client.rs`)**:
Fai in modo che il client catturi la stringa (es. `/ping`) e invii al server il JSON serializzato di `MessageType::Ping`.
3. **Gestisci la Logica nel Server (`server.rs`)**:
Nel blocco `match msg { ... }` del loop di lettura del client, aggiungi un ramo per `MessageType::Ping` e fai in modo che il server risponda inviando indietro `MessageType::Pong` sul canale del mittente (`tx`).
4. **Mostra l'Output nel Client**:
Nel loop di ricezione messaggi, aggiungi un match per `MessageType::Pong`.

---

## 7. Performance

Di seguito compilati i dati misurati dell'applicativo:

* **Dimensione file eseguibili:**
  * `server.exe`: `3.30 MB`
  * `client.exe`: `3.06 MB`
  * `gui_client.exe`: `11.5 MB`
* **CPU utilizzata:**
  * `server`: `0.01%`
  * `client`: `0.01%`
  * `gui_client`: `16.2%`

### Componenti PC di riferimento

* **CPU (modello):** `Intel(R) Core(TM) i5-8400 CPU @ 2.80GHz   2.81 GHz`
* **Numero di core / thread:** `6`
* **RAM totale:** `16,0 GB`
* **Sistema operativo:** `Windows 10`