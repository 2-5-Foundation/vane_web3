use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
pub use vane_crypto::*;
pub mod vane_crypto {
    use anyhow::anyhow;
    use base58::FromBase58;
    use curve25519_dalek::edwards::CompressedEdwardsY;
    use primitives::data_structure::{ChainSupported, Token};

    /// per the network selected verify that it makes sense cryptographically to have that account address bytes format
    pub fn verify_public_bytes(
        account: &str,
        token: Token,
        _network: ChainSupported,
    ) -> Result<ChainSupported, anyhow::Error> {
        match token {
            Token::Dot | Token::UsdtDot => {
                // check the byte length after removing prefix
                // remove the encoding scheme
                // check if it belongs to a ristretto group

                todo!()
            }
            Token::Bnb => {
                // check if it belongs to a point on Ecdsa secp256k1 curve
                // check the derivation path which is m/44'/60'/0'/0
                // check if it belongs to a point on Ecdsa secp256k1 curve !!! cannot do this as the public key is hashed
                // check if the account is 20 bytes
                if account.as_bytes().len() == 20 {
                    Ok(ChainSupported::Ethereum)
                } else {
                    Err(anyhow!("Not ethereum address"))
                }
            }
            Token::Sol | Token::UsdcSol | Token::UsdtSol => {
                // check if it belongs to a point on Ed25519 curve
                let bytes = account
                    .from_base58()
                    .map_err(|_| anyhow!("failed addr from base58"))?;
                let compressed_point = CompressedEdwardsY::from_slice(&bytes)
                    .map_err(|_| anyhow!("accounts bytes not 32"))?;
                if let Some(_) = compressed_point.decompress() {
                    Ok(ChainSupported::Solana)
                } else {
                    Err(anyhow!("not a valid ed25519 curve point"))
                }
            }
            Token::Eth | Token::UsdtEth | Token::UsdcEth => {
                // check if it belongs to a point on Ecdsa secp256k1 curve !!! cannot do this as the public key is hashed
                // check if the account is 20 bytes
                if account.as_bytes().len() == 42 {
                    /* 2 bytes for 0x and 40 bytes for rem as hex takes 2 character per bytes*/
                    Ok(ChainSupported::Ethereum)
                } else {
                    Err(anyhow!("Not ethereum address"))
                }
            }
        }
    }
}
