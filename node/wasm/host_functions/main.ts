import { hostCryptography } from "./cryptography";
import { hostNetworking } from "./networking";
import { hostLogging } from "./logging";

export const hostFunctions = {
    hostNetworking: hostNetworking,
    hostCryptography: hostCryptography,
    hostLogging: hostLogging,
}