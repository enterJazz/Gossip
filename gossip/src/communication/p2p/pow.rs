use blake3;
use log::error;
use rand::{self, RngCore};
use tokio::sync::{broadcast, mpsc};

/// Number of concurrent tasks used to calculate nonce
const N_TASKS: usize = 8;

/// Length of the challenge field in bytes
pub const CHALLENGE_LEN: usize = 4;
/// Length of the nonce field in bytes
pub const NONCE_LEN: usize = 4;

/// Length of the identity field in bytes
const ID_LEN: usize = 32;
/// Length of the generated blake3 hash
const HASH_LEN: usize = 32;

/// Generates a challenge for use in the Connection Challenge message
pub async fn generate_challenge() -> [u8; CHALLENGE_LEN] {
    generate_rand_slice().await
}

/// Generates a random byte slice of length NONCE_LEN
async fn generate_rand_slice() -> [u8; NONCE_LEN] {
    let mut rng = rand::thread_rng();
    let mut challenge = [0u8; CHALLENGE_LEN];
    rng.fill_bytes(&mut challenge);
    challenge
}

/// Solves the Proof of Work puzzle for a given source and target ID, a Connection Challenge supplied challenge wrt. a given difficulty,
/// This difficulty determines the length of the zero prefix in the response message hash
pub async fn generate_proof_of_work(
    src_id: [u8; ID_LEN],
    target_id: [u8; ID_LEN],
    challenge: [u8; CHALLENGE_LEN],
    difficulty: u8,
) -> [u8; NONCE_LEN] {
    let (mut close_tx, close_rx): (broadcast::Sender<()>, broadcast::Receiver<()>) =
        broadcast::channel(1);
    let (nonce_tx, mut nonce_rx): (
        mpsc::Sender<[u8; NONCE_LEN]>,
        mpsc::Receiver<[u8; NONCE_LEN]>,
    ) = mpsc::channel(1);

    for _ in 0..N_TASKS {
        let mut close_rx = close_tx.subscribe();
        let mut nonce_tx = nonce_tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = close_rx.recv() => {
                        break
                    },
                    candidate_nonce = generate_rand_slice() => {
                        let candidate_hash = calculate_hash(
                            src_id.clone(),
                            target_id.clone(),
                            challenge.clone(),
                            candidate_nonce.clone(),
                        ).await;
                        if check_hash_is_valid(&candidate_hash, difficulty).await {
                            if !nonce_tx.send(candidate_nonce).await.is_ok() {
                                error!("sending nonce failed");
                                break
                            };
                        }
                    }
                };
            }
        });
    }

    let nonce = nonce_rx.recv().await.unwrap();
    let _ = close_tx.send(()).unwrap_or_else(|e| {
        error!("closing tasks failed: {}", e);
        0
    });

    return nonce;
}

/// Validates if a given nonce along with other Connection Challenge parameters produces a valid hash.
/// The hash validity is determined by the hash difficulty, i.e., the expected length of the hash zero prefix.
pub async fn validate_nonce(
    src_id: [u8; ID_LEN],
    target_id: [u8; ID_LEN],
    challenge: [u8; CHALLENGE_LEN],
    nonce: [u8; NONCE_LEN],
    difficulty: u8,
) -> bool {
    let candidate_hash = calculate_hash(src_id, target_id, challenge, nonce).await;
    check_hash_is_valid(&candidate_hash, difficulty).await
}

/// Caluclates a hash with the given Connection Challenge Response parameters
async fn calculate_hash(
    src_id: [u8; ID_LEN],
    target_id: [u8; ID_LEN],
    challenge: [u8; CHALLENGE_LEN],
    nonce: [u8; NONCE_LEN],
) -> [u8; 32] {
    let mut hash_input = [0u8; ID_LEN * 2 + CHALLENGE_LEN + NONCE_LEN];

    hash_input[..ID_LEN].clone_from_slice(&src_id);
    hash_input[ID_LEN..ID_LEN * 2].clone_from_slice(&target_id);
    hash_input[ID_LEN * 2..ID_LEN * 2 + CHALLENGE_LEN].clone_from_slice(&challenge);
    hash_input[ID_LEN * 2 + CHALLENGE_LEN..ID_LEN * 2 + CHALLENGE_LEN + NONCE_LEN]
        .clone_from_slice(&nonce);

    let candidate_hash = blake3::hash(&hash_input);
    *candidate_hash.as_bytes()
}

/// Checks if a given hash is valid wrt. to a given difficulty, i.e., if the first n-bytes of a hash are equal to 0.
async fn check_hash_is_valid(hash: &[u8; HASH_LEN], difficulty: u8) -> bool {
    let zero_slice = [0u8; HASH_LEN];
    hash[0..(difficulty as usize)] == zero_slice[0..(difficulty as usize)]
}

#[cfg(test)]
mod tests {
    use rand::RngCore;

    use super::{generate_challenge, generate_proof_of_work, validate_nonce, ID_LEN, NONCE_LEN};

    #[tokio::test]
    async fn test_generate_challenge() {
        let challenge = generate_challenge().await;
        assert_eq!(challenge.len(), NONCE_LEN)
    }

    #[tokio::test]
    async fn test_generate_proof_of_work() {
        let mut rng = rand::thread_rng();
        let mut src_id = [0u8; ID_LEN];
        rng.fill_bytes(&mut src_id);
        let mut target_id = [0u8; ID_LEN];
        rng.fill_bytes(&mut target_id);
        let challenge = generate_challenge().await;
        let difficulty: u8 = 1;

        let nonce = generate_proof_of_work(
            src_id.clone(),
            target_id.clone(),
            challenge.clone(),
            difficulty,
        )
        .await;

        assert!(
            validate_nonce(
                src_id.clone(),
                target_id.clone(),
                challenge.clone(),
                nonce,
                difficulty,
            )
            .await
        )
    }
}
