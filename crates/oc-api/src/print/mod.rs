//! Ports of services/api/src/print/*.ts — print orders (A18): catalogue, state machine, persistence, payments,
//! fulfilment (Gelato, Printful, local print-ready files) and fulfiller webhooks.
//!
//! The order row is kept as the node-postgres row JSON (`SELECT * FROM print_orders`), which is what the routes
//! return; [`store::Order`] reads its fields.

pub mod catalog;
pub mod fulfil;
pub mod gelato;
pub mod local;
pub mod payment;
pub mod printful;
pub mod providers;
pub mod states;
pub mod store;
pub mod types;
pub mod updates;
