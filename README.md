# Ruggine

This repository contains the source code and documentation for "Ruggine," a Rust-based client-server chat application developed as an exam project. The project features a graphical user interface (GUI) and is fully containerized using Docker.

## Project Structure

The repository is organized into the following key components:

* **`ruggine/`**: The core Rust workspace containing the application code.


* **`src/bin/server.rs`**: The backend server binary handling client connections and message routing.


* **`src/bin/client.rs`**: A command-line interface (CLI) client.


* **`src/bin/gui_client.rs`**: The primary graphical client for end-users.




* **`assets/`**: Contains interface mockups and application screenshots, including:
* `GUI_Login.png`

* `GUI_Chats.png`

* `GUI_Message.png`

* `GUI_Management.png`



* **`manuals/`**: Comprehensive project documentation in Italian, including a User Manual (`Manuale_Utente.md`) and a Developer/Design Manual (`Manuale_Progettista.md`).


* **`Presentazione_Ruggine.pdf`**: The final presentation slides for the exam.



## Requirements

To run this project, you will need:

* [Rust and Cargo](https://www.rust-lang.org/) (configured via `Cargo.toml` and `Cargo.lock`)


* [Docker](https://www.docker.com/) and Docker Compose (optional, for containerized deployment)



## Getting Started

### Using Docker (Recommended)

The project includes a `Dockerfile` and a `docker-compose.yml` for easy setup and teardown.

*Option 1: Build and run the environment:*
```bash
docker-compose up --build

```

*Option 2: Pull the Server Image from Docker Hub:*

Bash
docker pull samvia/ruggine-server:latest
Build and run locally using Docker Compose:

Bash
docker-compose up --build



### Local Development (Cargo)

To run the components locally without Docker, navigate to the `ruggine` directory:

1. **Start the Server:**
```bash
cargo run --bin server

```


2. **Start the GUI Client:**
```bash
cargo run --bin gui_client

```



For a quick demonstration of the application's capabilities, you can also execute the provided `demo.bat` script located in the `ruggine` folder.

## Documentation

For detailed instructions on using the application or understanding its underlying architecture, please refer to the markdown files located in the `manuals/` directory.