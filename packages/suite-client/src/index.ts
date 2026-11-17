/**
 * `@suite-media/client` : client TypeScript du protocole Suite Media v1.
 *
 * - {@link SuiteClient} : client générique, pour n'importe quelle app de la suite.
 * - {@link StudioClient} : requêtes et événements de Maya Studio typés.
 * - {@link applyStudioEvent} : tient un état local à jour à partir des événements.
 */

export * from "./protocol.js";
export * from "./errors.js";
export * from "./backoff.js";
export * from "./client.js";
export * from "./studio.js";
