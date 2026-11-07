//! Données du serveur : les clients (clés d'API) et leurs compétitions.
//!
//! ```text
//! <data>/clients.json                     clients (clubs) et empreintes de leurs clés
//! <data>/users.json                       comptes (e-mail, mot de passe haché, club)
//! <data>/clients/<client>/<id>.cotcot     compétitions d'un client (format BellePoule)
//! ```
//! Un client ne voit que les compétitions de son dossier.

use std::collections::BTreeMap;
use std::path::PathBuf;

use bp_core::contest::Contest;

use crate::accounts::{Session, UserRecord};
use crate::auth::{Client, ClientRecord, KeyRecord};

pub struct Store {
    pub(crate) dir: PathBuf,
    clients: Vec<ClientRecord>,
    /// (client, compétition) → compétition.
    pub(crate) contests: BTreeMap<(String, String), Contest>,
    /// Organisation (pistes, arbitres) de chaque client.
    pub(crate) organisations: std::collections::HashMap<String, crate::organisation::Organisation>,
    /// Relais SmartPoule de chaque client.
    pub(crate) relays: std::collections::HashMap<String, crate::smartpoule::Relay>,
    /// Scores en direct (téléphones → écrans LivePoule) : club → piste → état.
    pub(crate) live: std::collections::HashMap<String, std::collections::HashMap<u32, crate::smartpoule::LiveBoard>>,
    /// Comptes utilisateurs.
    pub(crate) users: Vec<UserRecord>,
    /// Sessions ouvertes (empreinte du jeton → session), perdues au redémarrage.
    pub(crate) sessions: std::collections::HashMap<String, Session>,
    /// Échecs de connexion récents par e-mail : (nombre, premier échec).
    pub(crate) failures: std::collections::HashMap<String, (u32, std::time::Instant)>,
    /// Modèle de catégories générales de chaque club (relu à la demande).
    categories: std::cell::RefCell<std::collections::HashMap<String, bp_core::categories::CategoryModel>>,
    /// Épreuves publiées (compete.managecomp.org), lues au premier accès.
    pub(crate) publications: Option<Vec<crate::public::Publication>>,
    /// Événements publiés (épreuves ouvertes ensemble), lus au premier accès.
    pub(crate) public_events: Option<Vec<crate::public::PublicEvent>>,
}

impl Store {
    /// Ouvre le dossier de données (créé au besoin) et relit tout.
    pub fn open(dir: PathBuf) -> std::io::Result<Store> {
        std::fs::create_dir_all(dir.join("clients"))?;
        let clients: Vec<ClientRecord> = match std::fs::read_to_string(dir.join("clients.json")) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let users: Vec<UserRecord> = match std::fs::read_to_string(dir.join("users.json")) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let mut contests = BTreeMap::new();
        for client in &clients {
            let cdir = dir.join("clients").join(&client.id);
            let Ok(entries) = std::fs::read_dir(&cdir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("cotcot") {
                    continue;
                }
                let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
                let mut contest = Contest::new(None, false);
                match contest.load_file(&path) {
                    Ok(()) => {
                        contests.insert((client.id.clone(), id), contest);
                    }
                    Err(e) => tracing::warn!("{} : {e}", path.display()),
                }
            }
        }
        tracing::info!("{} client(s), {} compte(s), {} compétition(s) chargés depuis {}", clients.len(), users.len(), contests.len(), dir.display());
        Ok(Store {
            dir,
            clients,
            contests,
            organisations: Default::default(),
            relays: Default::default(),
            live: Default::default(),
            users,
            sessions: Default::default(),
            failures: Default::default(),
            categories: Default::default(),
            publications: None,
            public_events: None,
        })
    }

    // ------------------------------------------------------------- clients

    pub fn clients(&self) -> &[ClientRecord] {
        &self.clients
    }

    /// Le club d'une clé d'API (clé principale ou clé nommée).
    pub fn client_by_hash(&self, hash: &str) -> Option<Client> {
        self.clients
            .iter()
            .find(|c| c.key_hash == hash || c.keys.iter().any(|k| k.hash == hash))
            .map(|c| Client { id: c.id.clone(), name: c.name.clone() })
    }

    pub fn client(&self, id: &str) -> Option<Client> {
        self.clients.iter().find(|c| c.id == id).map(|c| Client { id: c.id.clone(), name: c.name.clone() })
    }

    pub fn count_for(&self, client: &str) -> usize {
        self.contests.keys().filter(|(c, _)| c == client).count()
    }

    /// Nouveau club ; `key_hash` vide : sans clé principale (club d'un compte).
    pub fn add_client(&mut self, name: &str, key_hash: &str) -> std::io::Result<String> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.clients.push(ClientRecord {
            id: id.clone(),
            name: name.to_string(),
            key_hash: key_hash.to_string(),
            created: chrono_now(),
            keys: Vec::new(),
        });
        std::fs::create_dir_all(self.dir.join("clients").join(&id))?;
        self.save_clients()?;
        Ok(id)
    }

    pub fn set_client_key(&mut self, id: &str, key_hash: &str) -> std::io::Result<Option<String>> {
        let Some(c) = self.clients.iter_mut().find(|c| c.id == id) else { return Ok(None) };
        c.key_hash = key_hash.to_string();
        let name = c.name.clone();
        self.save_clients()?;
        Ok(Some(name))
    }

    pub fn remove_client(&mut self, id: &str) -> std::io::Result<bool> {
        let before = self.clients.len();
        self.clients.retain(|c| c.id != id);
        if self.clients.len() == before {
            return Ok(false);
        }
        self.contests.retain(|(c, _), _| c != id);
        self.save_clients()?;
        Ok(true)
    }

    /// Clés nommées d'un club.
    pub fn keys(&self, client: &str) -> Vec<KeyRecord> {
        self.clients.iter().find(|c| c.id == client).map(|c| c.keys.clone()).unwrap_or_default()
    }

    pub fn add_key(&mut self, client: &str, name: &str, hash: &str) -> std::io::Result<Option<KeyRecord>> {
        let Some(c) = self.clients.iter_mut().find(|c| c.id == client) else { return Ok(None) };
        let key = KeyRecord { id: uuid::Uuid::new_v4().simple().to_string(), name: name.to_string(), hash: hash.to_string(), created: chrono_now() };
        c.keys.push(key.clone());
        self.save_clients()?;
        Ok(Some(key))
    }

    pub fn remove_key(&mut self, client: &str, key: &str) -> std::io::Result<bool> {
        let Some(c) = self.clients.iter_mut().find(|c| c.id == client) else { return Ok(false) };
        let before = c.keys.len();
        c.keys.retain(|k| k.id != key);
        if c.keys.len() == before {
            return Ok(false);
        }
        self.save_clients()?;
        Ok(true)
    }

    pub(crate) fn save_users(&self) -> std::io::Result<()> {
        let path = self.dir.join("users.json");
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.users).unwrap_or_default())?;
        std::fs::rename(tmp, path)
    }

    fn save_clients(&self) -> std::io::Result<()> {
        let path = self.dir.join("clients.json");
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.clients).unwrap_or_default())?;
        std::fs::rename(tmp, path)
    }

    // -------------------------------------------------------- compétitions

    // ------------------------------------------------ catégories générales

    // ------------------------------------------------ compétitions retirées de l'accueil

    fn hidden_path(&self, client: &Client) -> PathBuf {
        self.dir.join("clients").join(&client.id).join("hidden.json")
    }

    /// Compétitions retirées de l'accueil (conservées).
    pub fn hidden(&self, client: &Client) -> std::collections::HashSet<String> {
        std::fs::read(self.hidden_path(client)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn set_hidden(&self, client: &Client, id: &str, hidden: bool) -> std::io::Result<()> {
        let mut set = self.hidden(client);
        if hidden {
            set.insert(id.to_string());
        } else {
            set.remove(id);
        }
        let path = self.hidden_path(client);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec(&set)?)
    }

    pub fn categories_path(&self, client: &Client) -> PathBuf {
        self.dir.join("clients").join(&client.id).join("categories.json")
    }

    /// Le modèle de catégories du club devient le modèle actif du calcul
    /// (appelé à chaque accès aux compétitions du club, sous le verrou).
    fn activate_categories(&self, client: &Client) {
        let model = self
            .categories
            .borrow_mut()
            .entry(client.id.clone())
            .or_insert_with(|| bp_core::categories::CategoryStore::load(&self.categories_path(client)).current())
            .clone();
        bp_core::categories::set_active(model);
    }

    /// Modèles enregistrés du club changés : le modèle actif est relu.
    pub fn categories_changed(&self, client: &Client) {
        self.categories.borrow_mut().remove(&client.id);
        self.activate_categories(client);
    }

    pub fn list<'a>(&'a self, client: &'a Client) -> impl Iterator<Item = (&'a String, &'a Contest)> {
        self.activate_categories(client);
        self.contests.iter().filter(move |((c, _), _)| *c == client.id).map(|((_, id), contest)| (id, contest))
    }

    pub fn get(&self, client: &Client, id: &str) -> Option<&Contest> {
        self.activate_categories(client);
        self.contests.get(&(client.id.clone(), id.to_string()))
    }

    /// Accès en écriture sans enregistrement (lectures qui recalculent).
    pub fn get_mut(&mut self, client: &Client, id: &str) -> Option<&mut Contest> {
        self.activate_categories(client);
        self.contests.get_mut(&(client.id.clone(), id.to_string()))
    }

    /// Ajoute une compétition et l'enregistre ; renvoie son identifiant.
    pub fn insert(&mut self, client: &Client, contest: Contest) -> std::io::Result<String> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.contests.insert((client.id.clone(), id.clone()), contest);
        self.save(client, &id)?;
        Ok(id)
    }

    /// Modifie une compétition puis l'enregistre.
    pub fn update<R>(&mut self, client: &Client, id: &str, f: impl FnOnce(&mut Contest) -> R) -> Option<std::io::Result<R>> {
        self.activate_categories(client);
        let contest = self.contests.get_mut(&(client.id.clone(), id.to_string()))?;
        let result = f(contest);
        Some(self.save(client, id).map(|()| result))
    }

    pub fn remove(&mut self, client: &Client, id: &str) -> std::io::Result<bool> {
        let Some(contest) = self.contests.remove(&(client.id.clone(), id.to_string())) else { return Ok(false) };
        let path = self.path(client, id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        // Compétition supprimée : sa page publique aussi (et son événement s'il est vide).
        if let Some(public_id) = contest.public_id.as_deref() {
            self.unpublish(client, public_id)?;
        }
        Ok(true)
    }

    fn path(&self, client: &Client, id: &str) -> PathBuf {
        self.dir.join("clients").join(&client.id).join(format!("{id}.cotcot"))
    }

    pub(crate) fn save(&mut self, client: &Client, id: &str) -> std::io::Result<()> {
        let path = self.path(client, id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let Some(contest) = self.contests.get_mut(&(client.id.clone(), id.to_string())) else { return Ok(()) };
        let xml = contest.to_xml();
        let meta = crate::public::PublicMeta::of(contest);
        // Écriture atomique : fichier temporaire puis renommage.
        let tmp = path.with_extension("cotcot.tmp");
        std::fs::write(&tmp, &xml)?;
        std::fs::rename(tmp, path)?;
        // Épreuve publiée : la copie publique suit.
        self.refresh_publication(client, &meta, &xml);
        Ok(())
    }
}

/// Date et heure courantes (secondes depuis 1970, UTC).
pub(crate) fn chrono_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs}")
}
