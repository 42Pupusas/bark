

use std::borrow::{Borrow, BorrowMut};

use bitcoin::{psbt, sighash, taproot, Transaction, TxOut, Witness};
use bitcoin::secp256k1::{self, Keypair};

use crate::exit;


const PROP_KEY_PREFIX: &'static [u8] = "bark".as_bytes();

enum PropKey {
	ClaimInput = 1,
}

lazy_static::lazy_static! {
	static ref PROP_KEY_CLAIM_INPUT: psbt::raw::ProprietaryKey = psbt::raw::ProprietaryKey {
		prefix: PROP_KEY_PREFIX.to_vec(),
		subtype: PropKey::ClaimInput as u8,
		key: Vec::new(),
	};
}

//TODO(stevenroose) the "corrupt psbt" expects are only safe if all psbts stay
// within internal use, if we ever share them for communication or in a db,
// they need to return errors
pub trait PsbtInputExt: BorrowMut<psbt::Input> {
	fn set_claim_input(&mut self, input: &exit::ClaimInput) {
		self.borrow_mut().proprietary.insert(PROP_KEY_CLAIM_INPUT.clone(), input.encode());
	}

	fn get_claim_input(&self) -> Option<exit::ClaimInput> {
		self.borrow().proprietary.get(&*PROP_KEY_CLAIM_INPUT)
			.map(|e| exit::ClaimInput::decode(&e).expect("corrupt psbt"))
	}

	fn try_sign_claim_input(
		&mut self,
		secp: &secp256k1::Secp256k1<impl secp256k1::Signing>,
		sighash_cache: &mut sighash::SighashCache<impl Borrow<Transaction>>,
		prevouts: &sighash::Prevouts<impl Borrow<TxOut>>,
		input_idx: usize,
		vtxo_key: &Keypair,
	) {
		let claim = if let Some(c) = self.get_claim_input() {
			c
		} else {
			return;
		};

		// Now we need to sign for this.
		let exit_script = claim.spec.exit_clause();
		let leaf_hash = taproot::TapLeafHash::from_script(
			&exit_script,
			taproot::LeafVersion::TapScript,
		);
		let sighash = sighash_cache.taproot_script_spend_signature_hash(
			input_idx, prevouts, leaf_hash, sighash::TapSighashType::Default,
		).expect("all prevouts provided");

		assert_eq!(vtxo_key.public_key(), claim.spec.user_pubkey);
		let sig = secp.sign_schnorr(&sighash.into(), &vtxo_key);

		let cb = claim.spec.exit_taproot()
			.control_block(&(exit_script.clone(), taproot::LeafVersion::TapScript))
			.expect("script is in taproot");

		let wit = Witness::from_slice(
			&[&sig[..], exit_script.as_bytes(), &cb.serialize()],
		);
		debug_assert_eq!(bitcoin::Weight::from_wu(wit.size() as u64), claim.satisfaction_weight());
		self.borrow_mut().final_script_witness = Some(wit);

	}
}

impl PsbtInputExt for psbt::Input {}
