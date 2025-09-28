pub use vane_crypto::*;
pub mod vane_crypto {
    use anyhow::anyhow;
    use base58::FromBase58;
    use curve25519_dalek::edwards::CompressedEdwardsY;
    use primitives::data_structure::{ChainSupported, Token};

    // verify checksum of addresses
    pub fn verify_public_bytes(
        account: &str,
        token: &Token,
        network: ChainSupported,
    ) -> Result<ChainSupported, anyhow::Error> {
        match token {
            Token::Ethereum(_) | Token::Bnb(_) | Token::Optimism(_) | Token::Arbitrum(_) | Token::Polygon(_) | Token::Base(_) => {
                let token_network: ChainSupported = token.clone().into();
                if token_network != network {
                    return Err(anyhow!("Token network does not match the network"));
                }
                // EVM-compatible address verification
                let address: alloy::primitives::Address = account.parse()
                    .map_err(|e| anyhow!("Invalid EVM address format: {}", e))?;
                let eip55 = address.to_checksum(None);
                let returned_address = alloy_primitives::Address::parse_checksummed(&eip55, None).map_err(|e| anyhow!("Invalid EVM address format: {}", e))?;
                if **returned_address == **address {
                    Ok(token_network)
                } else {
                    Err(anyhow!("Invalid EVM address format"))
                }
            }
            Token::Tron(_) => {
                // TRON address verification
                todo!("TRON address verification not implemented yet")
            }
            Token::Solana(_) => {
                // Solana address verification
                todo!("Solana address verification not implemented yet")
            }
            Token::Bitcoin(_) => {
                // Bitcoin address verification
                todo!("Bitcoin address verification not implemented yet")
            }
            Token::Polkadot(_) => {
                // Polkadot address verification
                todo!("Polkadot address verification not implemented yet")
            }
        }
    }

    pub fn verify_route(
        sender_network: ChainSupported,
        receiver_network: ChainSupported,
    ) -> Result<(), anyhow::Error> {
        if sender_network == receiver_network {
            Ok(())
        } else {
            Err(anyhow!("currently complex cross chain route not supported"))
        }
    }
}
