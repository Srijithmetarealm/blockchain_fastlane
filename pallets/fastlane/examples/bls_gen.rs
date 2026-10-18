// Generates BLS12-381 (TinyBLS381) material that matches pallet-fastlane's
// on-chain verifier, for driving `register_bls_key` / `attest_bls` from scripts.
//
//   cargo run -p pallet-fastlane --example bls_gen -- <seed_hex32> <payload_id_hex32>
//
// Prints two hex strings on one line: "<public_key_hex> <signature_hex>".
// The public key is independent of the payload id; the signature is over
// Message::new(b"fastlane-attest", payload_id) — identical to the pallet.

use w3f_bls::{Message, SecretKeyVT, SerializableToBytes, TinyBLS381};

type Engine = TinyBLS381;

fn hexdec(s: &str) -> Vec<u8> {
	let s = s.strip_prefix("0x").unwrap_or(s);
	(0..s.len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
		.collect()
}

fn hexenc(b: &[u8]) -> String {
	let mut s = String::from("0x");
	for x in b {
		s.push_str(&format!("{:02x}", x));
	}
	s
}

fn main() {
	let args: Vec<String> = std::env::args().collect();
	let seed = hexdec(&args[1]);
	let payload_id = hexdec(&args[2]);

	let sk = SecretKeyVT::<Engine>::from_seed(&seed);
	let pubkey = sk.into_public().to_bytes();
	let msg = Message::new(b"fastlane-attest", &payload_id);
	let sig = sk.sign(&msg).to_bytes();

	println!("{} {}", hexenc(&pubkey), hexenc(&sig));
}
