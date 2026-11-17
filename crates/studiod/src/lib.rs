//! Moteur de Maya Studio (`studiod`) : boucle de rendu, sorties et API WebSocket.
//!
//! Exposé en bibliothèque pour le binaire `studiod` et les tests d'intégration.

pub mod api;
#[cfg(feature = "gstreamer")]
pub mod captures;
#[cfg(feature = "gstreamer")]
pub mod devices;
#[cfg(feature = "gstreamer")]
pub mod encoders;
pub mod engine;
pub mod media;
pub mod outputs;
pub mod server;
