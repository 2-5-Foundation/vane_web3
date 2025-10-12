<p align="center">
  <img src="https://github.com/user-attachments/assets/997cc05e-b56c-45e5-8d5c-42cdaec4a9c4" alt="Vane Logo" width="100" height="auto">
</p>


# vane_web3

A full sovereign custodian implementation of risk-free transaction sending for web3 users.

### What are we solving?

  - Losing funds due to wrong address input ( a huge pain currently in web3 as the action is irreversible after sending the transaction ).

  - Losings funds due to wrong network selection while sending the transaction.

At some point the address can be correct but the choice of the network can result to loss of funds.

### Our Solution

Vane acts as a safety net for web3 users.

----
DEMO POST

https://x.com/MrishoLukamba/status/1866162459800707165


[App:](https://www.vaneweb3.com)

Sentiments:

[LinkedIn](https://www.linkedin.com/posts/jake-edwards-27bb44155_context-httpslnkdineaf7iyik-in-the-ugcPost-7352443063161532416-vObV?utm_source=social_share_send&utm_medium=member_desktop_web&rcm=ACoAADJZPCsBacGm1-F9KIi-L8AC5ynOqG1i-PE)

[X](https://x.com/onukogufavour/status/1945121048459976760?s=46)


## Technical design


## Architecture Benefits

The code is designed to provide a secure layer for Web3 transactions by:

- Verifying receiver addresses
- Confirming transaction details with both parties
- Providing a safety net against wrong addresses or networks
- Supporting multiple blockchain networks
- Using P2P networking for secure communication


This architecture ensures that transactions are verified and confirmed by both parties before being submitted to the blockchain, removing the risk of sending funds to wrong addresses or networks.


![SCR-20241217-pikb](https://github.com/user-attachments/assets/f8c82fa4-2d2b-46d8-87bf-7c1a7f18cae1)


### Project Structure

```
vane_web3/
├── scripts/                    # All build/test scripts
│   ├── build-wasm-package.sh   # WASM package builder
│   ├── run_e2e.sh             # E2E test runner
│   ├── start-relay.ts         # Relay node starter
├── node/
│   ├── native/                # Pure native P2P node (outdated)
│   ├── relay/                 # Relay P2P node
│   └── wasm/                  # WebAssembly relay client
├── integration-test/
│   └── wasm-e2e-ts/           # Browser-based WASM test
└── primitives/                # Shared data structures
```

----
 
# Roadmap

 ### Technical

 - Programmable cryptographic proof, proving ownership of ones possession of the  device.
 - Cross chain transaction protection
 - Fishing signature protection via enclaves

### Clients

 - Farcaster mini app client
 - Telegram mini app client
 - Metamask snap client
