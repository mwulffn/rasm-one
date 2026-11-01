//! Checks that the oracle really runs vasm.

use std::error::Error;

use rasm_oracle::{Format, Vasm};

#[test]
fn test_assemble_nop_gives_4e71() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let out = vasm.assemble(" nop\n", "68000", Format::Bin)?;
    assert!(out.success, "{}", out.stderr);
    assert_eq!(out.bytes, [0x4E, 0x71]);
    Ok(())
}

#[test]
fn test_assemble_unsupported_instruction_is_rejected() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let out = vasm.assemble(" movec vbr,d0\n", "68000", Format::Bin)?;
    assert!(!out.success);
    assert!(out.stderr.contains("not supported"), "{}", out.stderr);
    Ok(())
}
