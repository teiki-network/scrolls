use pallas::ledger::traverse::MultiEraOutput;
use pallas::ledger::traverse::{Asset, MultiEraBlock, OutputRef};
use serde::Deserialize;

use crate::{crosscut, model, prelude::*};

#[derive(Deserialize)]
pub struct Config {
    pub key_prefix_handle_to_address: Option<String>,
    pub key_prefix_address_to_handles: Option<String>,
    pub policy_id_hex: String,
}

pub struct Reducer {
    config: Config,
    policy: crosscut::policies::RuntimePolicy,
}

impl Reducer {
    fn to_handle_name(&self, asset: Asset) -> Option<String> {
        match asset.policy_hex() {
            Some(policy_id) if policy_id.eq(&self.config.policy_id_hex) => match asset {
                Asset::NativeAsset(_, name, _) => String::from_utf8(name).ok(),
                _ => None,
            },
            _ => None,
        }
    }

    fn process_consumed_txo(
        &mut self,
        ctx: &model::BlockContext,
        input: &OutputRef,
        output: &mut super::OutputPort,
    ) -> Result<(), gasket::error::Error> {
        let txo = ctx.find_utxo(input).apply_policy(&self.policy).or_panic()?;

        let txo = match txo {
            Some(x) => x,
            None => return Ok(()),
        };

        let handle_names: Vec<_> = txo
            .non_ada_assets()
            .into_iter()
            .filter_map(|x| self.to_handle_name(x))
            .collect();

        if handle_names.is_empty() {
            return Ok(());
        }

        let address = txo.address().map(|addr| addr.to_string()).or_panic()?;

        let a2h_prefix = self.config.key_prefix_address_to_handles.as_deref();

        for handle in handle_names {
            log::debug!("handle consumed: {address} => ${handle}");

            let a2h_crdt = model::CRDTCommand::set_remove(a2h_prefix, &address, handle);
            output.send(a2h_crdt.into())?;
        }

        Ok(())
    }

    fn process_produced_txo(
        &mut self,
        txo: &MultiEraOutput,
        output: &mut super::OutputPort,
    ) -> Result<(), gasket::error::Error> {
        let handle_names: Vec<_> = txo
            .non_ada_assets()
            .into_iter()
            .filter_map(|x| self.to_handle_name(x))
            .collect();

        if handle_names.is_empty() {
            return Ok(());
        }

        let address = txo.address().map(|addr| addr.to_string()).or_panic()?;

        let h2a_prefix = self.config.key_prefix_handle_to_address.as_deref();
        let a2h_prefix = self.config.key_prefix_address_to_handles.as_deref();

        for handle in handle_names {
            log::debug!("handle produced: ${handle} => {address}");

            let h2a_crdt =
                model::CRDTCommand::any_write_wins(h2a_prefix, &handle, address.clone());
            output.send(h2a_crdt.into())?;

            let a2h_crdt = model::CRDTCommand::set_add(a2h_prefix, &address, handle);
            output.send(a2h_crdt.into())?;
        }

        Ok(())
    }

    pub fn reduce_block<'b>(
        &mut self,
        block: &'b MultiEraBlock<'b>,
        ctx: &model::BlockContext,
        output: &mut super::OutputPort,
    ) -> Result<(), gasket::error::Error> {
        for tx in block.txs().into_iter() {
            for consumed in tx.consumes().iter().map(|i| i.output_ref()) {
                self.process_consumed_txo(&ctx, &consumed, output)?;
            }

            for (_, produced) in tx.produces() {
                self.process_produced_txo(&produced, output)?;
            }
        }

        Ok(())
    }
}

impl Config {
    pub fn plugin(self, policy: &crosscut::policies::RuntimePolicy) -> super::Reducer {
        let reducer = Reducer {
            config: self,
            policy: policy.clone(),
        };

        super::Reducer::AdaHandle(reducer)
    }
}
