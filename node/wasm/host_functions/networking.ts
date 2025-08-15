export const hostNetworking = {
    submitTx(tx: any): Promise<Uint8Array> {
      console.log("Submitting transaction:", tx);
      return Promise.resolve(new Uint8Array(32).fill(0));
    },
  
    createTx(tx: any): Promise<void> {
      console.log("Creating transaction:", tx);
      return Promise.resolve();
    },
  };