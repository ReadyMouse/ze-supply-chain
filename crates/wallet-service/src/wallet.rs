//! The wallet actor: owns the zcash_client_sqlite WalletDb, the lightwalletd
//! connection, and the record batch queue. Everything that touches the spending
//! key happens inside this module.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use secrecy::SecretVec;
use tokio::sync::{mpsc, oneshot};
use tonic::transport::{Channel, ClientTlsConfig};
use uuid::Uuid;

use zcash_client_backend::data_api::wallet::{
    create_proposed_transactions, input_selection::GreedyInputSelector, propose_transfer,
    ConfirmationsPolicy, SpendingKeys,
};
use zcash_client_backend::data_api::{Account as _, AccountBirthday, WalletRead, WalletWrite};
use zcash_client_backend::fees::{zip317::SingleOutputChangeStrategy, DustOutputPolicy};
use zcash_client_backend::proto::service::{
    compact_tx_streamer_client::CompactTxStreamerClient, BlockId, ChainSpec, RawTransaction,
};
use zcash_client_backend::sync;
use zcash_client_backend::wallet::OvkPolicy;
use zcash_client_sqlite::{util::SystemClock, wallet::init::init_wallet_db, AccountUuid, WalletDb};
use zcash_keys::keys::UnifiedAddressRequest;
use zcash_primitives::transaction::fees::zip317::FeeRule as Zip317FeeRule;
use zcash_primitives::transaction::{Transaction, TxId};
use zcash_protocol::consensus::{self, MAIN_NETWORK};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zcash_protocol::ShieldedProtocol;
use zip321::{Payment, TransactionRequest};

use memo_schema::Record;

use crate::block_cache::MemBlockCache;
use crate::config::Config;

/// Value attached to each memo-carrying output. ZIP 317 marginal fee is 5000
/// zatoshis; anything at or above that is not dust.
const RECORD_OUTPUT_ZAT: u64 = 10_000;
const SYNC_BATCH_SIZE: u32 = 1_000;

pub struct QueuedRecord {
    pub submission_id: Uuid,
    pub user_index: u32,
    pub record: Record,
}

pub enum Command {
    /// Create (or fetch) the account for `user_index` and return its address.
    EnsureAccount {
        user_index: u32,
        reply: oneshot::Sender<Result<String>>,
    },
    /// Queue a record for the next batch.
    Enqueue {
        record: QueuedRecord,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Flush the current batch immediately.
    ProcessBatch {
        reply: oneshot::Sender<Result<Vec<(Uuid, String)>>>,
    },
    /// Split treasury funds into `parts` equal notes so batches don't
    /// serialize behind a single unconfirmed change output.
    SplitNotes {
        parts: u32,
        zat_per_part: u64,
        reply: oneshot::Sender<Result<String>>,
    },
    Status {
        reply: oneshot::Sender<Result<WalletStatus>>,
    },
}

#[derive(serde::Serialize, Clone)]
pub struct WalletStatus {
    pub chain_tip: Option<u32>,
    pub scanned_height: Option<u32>,
    pub balance_zat: u64,
    pub spendable_zat: u64,
    pub queued_records: usize,
    pub org_address: String,
}

#[derive(Clone)]
pub struct WalletHandle {
    tx: mpsc::Sender<Command>,
}

impl WalletHandle {
    pub async fn ensure_account(&self, user_index: u32) -> Result<String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::EnsureAccount { user_index, reply })
            .await
            .map_err(|_| anyhow!("wallet actor gone"))?;
        rx.await.map_err(|_| anyhow!("wallet actor dropped reply"))?
    }

    pub async fn enqueue(&self, record: QueuedRecord) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::Enqueue { record, reply })
            .await
            .map_err(|_| anyhow!("wallet actor gone"))?;
        rx.await.map_err(|_| anyhow!("wallet actor dropped reply"))?
    }

    pub async fn process_batch(&self) -> Result<Vec<(Uuid, String)>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::ProcessBatch { reply })
            .await
            .map_err(|_| anyhow!("wallet actor gone"))?;
        rx.await.map_err(|_| anyhow!("wallet actor dropped reply"))?
    }

    pub async fn split_notes(&self, parts: u32, zat_per_part: u64) -> Result<String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::SplitNotes {
                parts,
                zat_per_part,
                reply,
            })
            .await
            .map_err(|_| anyhow!("wallet actor gone"))?;
        rx.await.map_err(|_| anyhow!("wallet actor dropped reply"))?
    }

    pub async fn status(&self) -> Result<WalletStatus> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::Status { reply })
            .await
            .map_err(|_| anyhow!("wallet actor gone"))?;
        rx.await.map_err(|_| anyhow!("wallet actor dropped reply"))?
    }
}

type Db = WalletDb<rusqlite::Connection, consensus::MainNetwork, SystemClock, rand::rngs::OsRng>;

pub struct WalletActor {
    cfg: Config,
    db: Db,
    seed: SecretVec<u8>,
    client: CompactTxStreamerClient<Channel>,
    cache: MemBlockCache,
    prover: zcash_proofs::prover::LocalTxProver,
    /// user_index -> (account uuid, encoded UA)
    accounts: HashMap<u32, (AccountUuid, String)>,
    queue: Vec<QueuedRecord>,
    oldest_queued_at: Option<Instant>,
    rx: mpsc::Receiver<Command>,
}

pub async fn connect_lightwalletd(url: &str) -> Result<CompactTxStreamerClient<Channel>> {
    let mut endpoint = Channel::from_shared(url.to_string()).context("bad LIGHTWALLETD_URL")?;
    if url.starts_with("https://") {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().with_webpki_roots())
            .context("tls config")?;
    }
    let channel = endpoint.connect().await.context("connect lightwalletd")?;
    Ok(CompactTxStreamerClient::new(channel)
        .max_decoding_message_size(64 * 1024 * 1024))
}

pub fn seed_from_phrase(phrase: &str) -> Result<SecretVec<u8>> {
    let mnemonic = <bip39::Mnemonic as std::str::FromStr>::from_str(phrase.trim())
        .context("invalid WALLET_SEED_PHRASE mnemonic")?;
    Ok(SecretVec::new(mnemonic.to_seed("").to_vec()))
}

impl WalletActor {
    pub async fn spawn(cfg: Config) -> Result<WalletHandle> {
        let seed = seed_from_phrase(&cfg.seed_phrase)?;

        if let Some(dir) = Path::new(&cfg.wallet_db_path).parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let mut db = WalletDb::for_path(
            &cfg.wallet_db_path,
            MAIN_NETWORK,
            SystemClock,
            rand::rngs::OsRng,
        )
        .context("open wallet db")?;
        init_wallet_db(&mut db, None).map_err(|e| anyhow!("init wallet db: {e}"))?;

        let mut client = connect_lightwalletd(&cfg.lightwalletd_url).await?;

        tracing::info!("fetching sapling parameters (first run downloads ~50MB)");
        let prover = match zcash_proofs::prover::LocalTxProver::with_default_location() {
            Some(p) => p,
            None => {
                let paths =
                    zcash_proofs::download_sapling_parameters(None).context("download params")?;
                zcash_proofs::prover::LocalTxProver::new(&paths.spend, &paths.output)
            }
        };

        let (tx, rx) = mpsc::channel(64);
        let mut actor = WalletActor {
            cfg,
            db,
            seed,
            client: client.clone(),
            cache: MemBlockCache::new(),
            prover,
            accounts: HashMap::new(),
            queue: Vec::new(),
            oldest_queued_at: None,
            rx,
        };

        // Make sure the treasury account (index 0) exists before serving requests.
        actor.ensure_account(0).await?;
        // Touch the connection so a bad endpoint fails fast.
        let _ = client.get_latest_block(ChainSpec {}).await?;

        tokio::spawn(async move { actor.run().await });
        Ok(WalletHandle { tx })
    }

    async fn run(&mut self) {
        let mut sync_tick = tokio::time::interval(Duration::from_secs(20));
        sync_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                cmd = self.rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    self.handle(cmd).await;
                }
                _ = sync_tick.tick() => {
                    if let Err(e) = self.sync_once().await {
                        tracing::warn!("sync error: {e:#}");
                    }
                    if self.batch_due() {
                        match self.flush_batch().await {
                            Ok(sent) if !sent.is_empty() => {
                                tracing::info!("batch broadcast: {} records", sent.len());
                            }
                            Ok(_) => {}
                            Err(e) => tracing::error!("batch flush failed: {e:#}"),
                        }
                    }
                }
            }
        }
    }

    async fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::EnsureAccount { user_index, reply } => {
                let _ = reply.send(self.ensure_account(user_index).await);
            }
            Command::Enqueue { record, reply } => {
                if self.oldest_queued_at.is_none() {
                    self.oldest_queued_at = Some(Instant::now());
                }
                self.queue.push(record);
                let _ = reply.send(Ok(()));
            }
            Command::ProcessBatch { reply } => {
                let _ = reply.send(self.flush_batch().await);
            }
            Command::SplitNotes {
                parts,
                zat_per_part,
                reply,
            } => {
                let _ = reply.send(self.split_notes(parts, zat_per_part).await);
            }
            Command::Status { reply } => {
                let _ = reply.send(self.status().await);
            }
        }
    }

    fn batch_due(&self) -> bool {
        if self.queue.is_empty() {
            return false;
        }
        self.queue.len() >= self.cfg.batch_max_records
            || self
                .oldest_queued_at
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(self.cfg.batch_max_age_secs))
    }

    /// Create or load the ZIP 32 account for a user index and return its default address.
    async fn ensure_account(&mut self, user_index: u32) -> Result<String> {
        if let Some((_, addr)) = self.accounts.get(&user_index) {
            return Ok(addr.clone());
        }

        // Already in the wallet db (e.g. process restart)?
        for account_id in self.db.get_account_ids().map_err(|e| anyhow!("{e}"))? {
            let account = self
                .db
                .get_account(account_id)
                .map_err(|e| anyhow!("{e}"))?
                .ok_or_else(|| anyhow!("account vanished"))?;
            if let Some(idx) = account_zip32_index(&account) {
                let addr = self.default_address(&account)?;
                self.accounts.insert(idx, (account.id(), addr));
            }
        }
        if let Some((_, addr)) = self.accounts.get(&user_index) {
            return Ok(addr.clone());
        }

        // New account: birthday at the configured wallet birthday so re-derivation
        // from seed always covers the full org history.
        let birthday_height = self.cfg.birthday;
        let treestate = self
            .client
            .get_tree_state(BlockId {
                height: birthday_height.saturating_sub(1) as u64,
                hash: vec![],
            })
            .await
            .context("get_tree_state")?
            .into_inner();
        let birthday = AccountBirthday::from_treestate(treestate, None)
            .map_err(|_| anyhow!("invalid treestate for birthday height {birthday_height}"))?;

        let zip32_index =
            zip32::AccountId::try_from(user_index).map_err(|_| anyhow!("bad user index"))?;
        let (account, _usk) = self
            .db
            .import_account_hd(
                &format!("user-{user_index}"),
                &self.seed,
                zip32_index,
                &birthday,
                Some("ze-supply-chain"),
            )
            .map_err(|e| anyhow!("import_account_hd: {e}"))?;

        let addr = self.default_address(&account)?;
        self.accounts
            .insert(user_index, (account.id(), addr.clone()));
        Ok(addr)
    }

    fn default_address(
        &self,
        account: &<Db as WalletRead>::Account,
    ) -> Result<String> {
        let ufvk = account
            .ufvk()
            .ok_or_else(|| anyhow!("account has no UFVK"))?;
        let (ua, _) = ufvk
            .default_address(UnifiedAddressRequest::AllAvailableKeys)
            .map_err(|e| anyhow!("default_address: {e}"))?;
        Ok(ua.encode(&MAIN_NETWORK))
    }

    async fn sync_once(&mut self) -> Result<()> {
        sync::run(
            &mut self.client,
            &MAIN_NETWORK,
            &self.cache,
            &mut self.db,
            SYNC_BATCH_SIZE,
        )
        .await
        .map_err(|e| anyhow!("sync: {e}"))
    }

    /// Build, prove, and broadcast one send-many transaction carrying every
    /// queued record as a memo output to its user's address.
    async fn flush_batch(&mut self) -> Result<Vec<(Uuid, String)>> {
        if self.queue.is_empty() {
            return Ok(vec![]);
        }
        // Make sure we're scanned to the tip so note selection sees confirmed funds.
        self.sync_once().await?;

        let batch: Vec<QueuedRecord> = std::mem::take(&mut self.queue);
        self.oldest_queued_at = None;

        let result = self.send_batch(&batch).await;
        match result {
            Ok(txid) => {
                let txid_hex = txid.to_string();
                Ok(batch
                    .iter()
                    .map(|r| (r.submission_id, txid_hex.clone()))
                    .collect())
            }
            Err(e) => {
                // Put the records back so nothing is lost.
                self.queue = batch;
                self.oldest_queued_at = Some(Instant::now());
                Err(e)
            }
        }
    }

    async fn send_batch(&mut self, batch: &[QueuedRecord]) -> Result<TxId> {
        let mut payments = Vec::with_capacity(batch.len());
        for rec in batch {
            let addr = self.ensure_account(rec.user_index).await?;
            let zaddr = zcash_address::ZcashAddress::try_from_encoded(&addr)
                .map_err(|e| anyhow!("address parse: {e}"))?;
            let memo_bytes = memo_schema::encode_memo(&rec.record)?;
            let memo = MemoBytes::from_bytes(&memo_bytes).map_err(|e| anyhow!("memo: {e}"))?;
            let payment = Payment::new(
                zaddr,
                Some(Zatoshis::const_from_u64(RECORD_OUTPUT_ZAT)),
                Some(memo),
                None,
                None,
                vec![],
            )
            .map_err(|e| anyhow!("payment construction failed: {e}"))?;
            payments.push(payment);
        }
        let request =
            TransactionRequest::new(payments).map_err(|e| anyhow!("tx request: {e:?}"))?;
        self.send_request(request).await
    }

    /// Self-transfer that splits treasury funds into `parts` equal notes.
    async fn split_notes(&mut self, parts: u32, zat_per_part: u64) -> Result<String> {
        if parts == 0 || parts > 50 {
            return Err(anyhow!("parts must be between 1 and 50"));
        }
        self.sync_once().await?;

        let addr = self.ensure_account(0).await?;
        let zaddr = zcash_address::ZcashAddress::try_from_encoded(&addr)
            .map_err(|e| anyhow!("address parse: {e}"))?;
        let amount = Zatoshis::from_u64(zat_per_part).map_err(|_| anyhow!("bad amount"))?;
        let payments = (0..parts)
            .map(|_| Payment::without_memo(zaddr.clone(), amount))
            .collect::<Vec<_>>();
        let request =
            TransactionRequest::new(payments).map_err(|e| anyhow!("tx request: {e:?}"))?;
        let txid = self.send_request(request).await?;
        Ok(txid.to_string())
    }

    /// Propose, prove, store, and broadcast a transaction request spending
    /// from the treasury account.
    async fn send_request(&mut self, request: TransactionRequest) -> Result<TxId> {
        let treasury = self
            .accounts
            .get(&0)
            .ok_or_else(|| anyhow!("treasury account missing"))?
            .0;

        let input_selector = GreedyInputSelector::<Db>::new();
        let change_strategy = SingleOutputChangeStrategy::<Zip317FeeRule, Db>::new(
            Zip317FeeRule::standard(),
            None,
            ShieldedProtocol::Orchard,
            DustOutputPolicy::default(),
        );
        let confirmations =
            ConfirmationsPolicy::new_symmetrical(NonZeroU32::new(3).expect("nonzero"));

        let proposal = propose_transfer::<_, _, _, _, std::convert::Infallible>(
            &mut self.db,
            &MAIN_NETWORK,
            treasury,
            &input_selector,
            &change_strategy,
            request,
            confirmations,
            None,
        )
        .map_err(|e| anyhow!("propose: {e}"))?;

        // Re-derive the treasury USK from seed (never stored).
        let usk = zcash_keys::keys::UnifiedSpendingKey::from_seed(
            &MAIN_NETWORK,
            secrecy::ExposeSecret::expose_secret(&self.seed),
            zip32::AccountId::ZERO,
        )
        .map_err(|e| anyhow!("usk: {e}"))?;

        let txids = create_proposed_transactions::<
            _,
            _,
            std::convert::Infallible,
            _,
            zcash_primitives::transaction::fees::zip317::FeeError,
            _,
        >(
            &mut self.db,
            &MAIN_NETWORK,
            &self.prover,
            &self.prover,
            &SpendingKeys::from_unified_spending_key(usk),
            OvkPolicy::Sender,
            &proposal,
            None,
        )
        .map_err(|e| anyhow!("create tx: {e}"))?;

        // Broadcast every step (normally exactly one tx).
        let mut last = None;
        for txid in txids.iter() {
            let raw = self.raw_transaction(txid)?;
            let resp = self
                .client
                .send_transaction(RawTransaction {
                    data: raw,
                    height: 0,
                })
                .await
                .context("send_transaction")?
                .into_inner();
            if resp.error_code != 0 {
                return Err(anyhow!("broadcast rejected: {}", resp.error_message));
            }
            tracing::info!("broadcast tx {txid}");
            last = Some(*txid);
        }
        last.ok_or_else(|| anyhow!("no transactions created"))
    }

    fn raw_transaction(&mut self, txid: &TxId) -> Result<Vec<u8>> {
        let tx: Transaction = self
            .db
            .get_transaction(*txid)
            .map_err(|e| anyhow!("{e}"))?
            .ok_or_else(|| anyhow!("created tx not found in wallet db"))?;
        let mut buf = Vec::new();
        tx.write(&mut buf).context("serialize tx")?;
        Ok(buf)
    }

    async fn status(&mut self) -> Result<WalletStatus> {
        let tip = self
            .client
            .get_latest_block(ChainSpec {})
            .await
            .ok()
            .map(|b| b.into_inner().height as u32);
        let scanned = self
            .db
            .block_max_scanned()
            .map_err(|e| anyhow!("{e}"))?
            .map(|m| u32::from(m.block_height()));

        let summary = self
            .db
            .get_wallet_summary(ConfirmationsPolicy::new_symmetrical(
                NonZeroU32::new(3).expect("nonzero"),
            ))
            .map_err(|e| anyhow!("{e}"))?;
        let balance_zat = summary
            .as_ref()
            .map(|s| {
                s.account_balances()
                    .values()
                    .map(|b| u64::from(b.total()))
                    .sum()
            })
            .unwrap_or(0);
        let spendable_zat = summary
            .as_ref()
            .map(|s| {
                s.account_balances()
                    .values()
                    .map(|b| {
                        u64::from(b.sapling_balance().spendable_value())
                            + u64::from(b.orchard_balance().spendable_value())
                    })
                    .sum()
            })
            .unwrap_or(0);

        let org_address = self
            .accounts
            .get(&0)
            .map(|(_, a)| a.clone())
            .unwrap_or_default();

        Ok(WalletStatus {
            chain_tip: tip,
            scanned_height: scanned,
            balance_zat,
            spendable_zat,
            queued_records: self.queue.len(),
            org_address,
        })
    }
}

/// Extract the ZIP 32 account index from an account's source metadata.
fn account_zip32_index(account: &impl zcash_client_backend::data_api::Account) -> Option<u32> {
    match account.source() {
        zcash_client_backend::data_api::AccountSource::Derived { derivation, .. } => {
            Some(u32::from(derivation.account_index()))
        }
        _ => None,
    }
}