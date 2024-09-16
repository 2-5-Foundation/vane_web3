
pub use VaneCrypto::*;
pub mod VaneCrypto {
    use primitives::data_structure::{ChainSupported, Token};

    // per the network selected verify that it makes sense cryptographically to have that account address bytes format
    pub fn verify_public_bytes(account: &str, token: Token, network: ChainSupported) -> Result<ChainSupported,anyhow::Error>{
        match token {
            Token::Dot => {
                // check the byte length after removing prefix
                // remove the encoding scheme
                // check if it belongs to a ristretto group
                todo!()
            }
            Token::Bnb => {
                // check if it belongs to a point on Ecdsa secp256k1 curve
                // check the derivation path which is m/44'/60'/0'/0
                todo!()
            }
            Token::Sol | Token::UsdcSol | Token::UsdtSol => {
                // check if it belongs to a point on Ed25519 curve
                todo!()
            }
            Token::Eth | Token::UsdtEth | Token::UsdcEth=> {
                // check if it belongs to a point on Ecdsa secp256k1 curve
                todo!()
            }
        }
    }
}