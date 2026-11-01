//! Compares MMU instructions with vasm -no-opt, beyond the sweep's samples.

use std::error::Error;

use rasm_one::{assemble, write_bin, Assembly, Cpu, MemoryProvider, Options, Target};
use rasm_oracle::{Format, Vasm};

fn ours(source: &str, cpu: Cpu, mmu_68851: bool) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    let mut options = Options::unoptimised("t.s");
    options.cpu = cpu;
    options.mmu_68851 = mmu_68851;
    options.target = Target::RawBinary;
    assemble(&options, &files)
}

const ON_68851: &[&str] = &[
    " pflusha",
    " pflush #1,#2",
    " pflush #15,#15",
    " ploadw #15,(a0)",
    " ptestr #15,(a0),#7",
    " pflush #1,#2,(a0)",
    " pflush d0,#2",
    " pflush sfc,#2",
    " pflush dfc,#2,(a0)",
    " pflushs #1,#2",
    " pflushs #1,#2,(a0)",
    " pflushr (a0)",
    " pflushr #1",
    " ploadr #1,(a0)",
    " ploadw #1,(a0)",
    " ploadr sfc,(a0)",
    " ploadw d1,4(a0)",
    " pmove tc,(a0)",
    " pmove (a0),tc",
    " pmove.l tc,(a0)",
    " pmove crp,(a0)",
    " pmove.q (a0),crp",
    " pmove srp,(a0)",
    " pmove drp,(a0)",
    " pmove cal,(a0)",
    " pmove val,(a0)",
    " pmove scc,(a0)",
    " pmove ac,(a0)",
    " pmove psr,(a0)",
    " pmove pcsr,(a0)",
    " pmove mmusr,(a0)",
    " pmove bad0,(a0)",
    " pmove bac7,(a0)",
    " pmove (a0),bad1",
    " pmove tc,d1",
    " pmove #1,tc",
    " pmove #1,crp",
    " pmove #1,val",
    " pmove (a0)+,srp",
    " ptestr #1,(a0),#2",
    " ptestw #1,(a0),#7",
    " ptestr #1,(a0),#2,a1",
    " ptestr d0,(a0),#2",
    " ptestr sfc,(a0),#2",
    " pvalid val,(a0)",
    " pvalid a0,(a0)",
    " psave (a0)",
    " prestore (a0)",
    " mc68851\n pflusha",
    " machine 68851\n pflusha",
];

const CONDITIONS: &[&str] = &[
    "bs", "bc", "ls", "lc", "ss", "sc", "as", "ac", "ws", "wc", "is", "ic", "gs", "gc", "cs", "cc",
];

const ON_68030: &[&str] = &[
    " pflusha",
    " pflush #1,#2",
    " pflush #7,#7",
    " ploadr #7,(a0)",
    " ptestw #7,(a0),#7",
    " pmove tc,(a0)",
    " pmove tt0,(a0)",
    " pmove tt1,4(a0)",
    " pmove (a0),tt0",
    " pmove mmusr,(a0)",
    " pmovefd (a0),tc",
    " pmovefd (a0),tt1",
    " pmovefd (a0),crp",
    " ptestr #1,(a0),#2",
];

const ON_68040: &[&str] = &[
    " pflusha",
    " pflushan",
    " pflushn (a0)",
    " pflush (a7)",
    " ptestr (a0)",
    " ptestw (a1)",
];

const ON_68060: &[&str] = &[
    " pflusha",
    " pflushan",
    " pflushn (a0)",
    " pflush (a0)",
    " plpar (a0)",
    " plpaw (a1)",
];

const REJECTED_ON_68851: &[&str] = &[
    " pmove.l crp,(a0)",
    " pmove.w tc,(a0)",
    " pmove.b psr,(a0)",
    " pmove (a0),pcsr",
    " pmove srp,d0",
    " pmove d0,srp",
    " pmove tt0,(a0)",
    " pmovefd (a0),tc",
    " pvalid tc,(a0)",
    " pflush #16,#2",
    " pflush #1,#16",
    " ploadr #16,(a0)",
    " ptestr #16,(a0),#2",
    " ptestr #1,(a0),#8",
];

const REJECTED_ON_68030: &[&str] = &[
    " pflush #8,#2",
    " pflush #1,#8",
    " ploadr #8,(a0)",
    " ptestr #8,(a0),#2",
    " ptestr #1,(a0),#8",
    " pmove drp,(a0)",
    " pvalid val,(a0)",
];

/// Sources and machines vasm rejects where the PRM allows them.
const VALID_DESPITE_VASM: &[&str] = &[" ploadr sfc,(a0)", " ploadw dfc,(a0)"];

#[test]
fn test_mmu_forms_match_vasm_bytes() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let conditions: Vec<String> = CONDITIONS
        .iter()
        .flat_map(|c| {
            [
                format!(" pb{c} *+6"),
                format!(" pb{c}.l *+8"),
                format!(" pdb{c} d1,*+8"),
                format!(" ps{c} (a1)"),
                format!(" ptrap{c}"),
                format!(" ptrap{c}.w #2"),
                format!(" ptrap{c}.l #2"),
            ]
        })
        .collect();
    let on_68851 = ON_68851
        .iter()
        .map(|source| source.to_string())
        .chain(conditions);
    let mut cases: Vec<(String, &str, Cpu, bool, &[&str])> = Vec::new();
    for source in on_68851 {
        cases.push((source, "68020", Cpu::M68020, true, &["-m68851"]));
    }
    let others: [(&[&str], &str, Cpu); 3] = [
        (ON_68030, "68030", Cpu::M68030),
        (ON_68040, "68040", Cpu::M68040),
        (ON_68060, "68060", Cpu::M68060),
    ];
    for (sources, name, cpu) in others {
        for source in sources {
            cases.push((source.to_string(), name, cpu, false, &[]));
        }
    }
    let mut failures = Vec::new();
    for (source, name, cpu, mmu_68851, args) in cases {
        let source = format!("{source}\n");
        let theirs = vasm.assemble_with_args(&source, name, Format::Bin, args)?;
        if !theirs.success {
            failures.push(format!(
                "vasm rejected {source:?} on {name}: {}",
                theirs.stderr
            ));
            continue;
        }
        let assembly = ours(&source, cpu, mmu_68851);
        match write_bin(&assembly) {
            Ok(bytes) if bytes == theirs.bytes => {}
            Ok(bytes) => failures.push(format!(
                "{source:?} on {name}\n  ours {bytes:02x?}\n  vasm {:02x?}",
                theirs.bytes
            )),
            Err(error) => failures.push(format!("{source:?}: {error}: {:?}", assembly.diagnostics)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_mmu_rejected_forms_match_vasm() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in REJECTED_ON_68851 {
        let source = format!("{source}\n");
        let theirs = vasm.assemble_with_args(&source, "68020", Format::Bin, &["-m68851"])?;
        if theirs.success {
            failures.push(format!("vasm accepted {source:?}"));
        }
        if !ours(&source, Cpu::M68020, true).has_errors() {
            failures.push(format!("we accepted {source:?}"));
        }
    }
    for source in REJECTED_ON_68030 {
        let source = format!("{source}\n");
        if vasm.assemble(&source, "68030", Format::Bin)?.success {
            failures.push(format!("vasm accepted {source:?} on 68030"));
        }
        if !ours(&source, Cpu::M68030, false).has_errors() {
            failures.push(format!("we accepted {source:?} on 68030"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_mmu_function_codes_valid_on_68030_despite_vasm() {
    for source in VALID_DESPITE_VASM {
        let source = format!("{source}\n");
        assert!(
            !ours(&source, Cpu::M68030, false).has_errors(),
            "{source:?}"
        );
    }
}

#[test]
fn test_mmu_availability_by_machine() {
    let accepts = |source: &str, cpu, mmu| !ours(source, cpu, mmu).has_errors();
    // The shared set needs the 68030 or a 68851.
    assert!(!accepts(" pflush #1,#2\n", Cpu::M68020, false));
    assert!(accepts(" pflush #1,#2\n", Cpu::M68020, true));
    assert!(accepts(" pflush #1,#2\n", Cpu::M68030, false));
    assert!(!accepts(" pflush #1,#2\n", Cpu::M68040, false));
    // Some are only on a 68851.
    assert!(!accepts(" pvalid val,(a0)\n", Cpu::M68030, false));
    assert!(accepts(" pvalid val,(a0)\n", Cpu::M68030, true));
    // Others are only on one CPU.
    assert!(accepts(" ptestr (a0)\n", Cpu::M68040, false));
    assert!(!accepts(" ptestr (a0)\n", Cpu::M68060, false));
    assert!(accepts(" plpar (a0)\n", Cpu::M68060, false));
    assert!(!accepts(" plpar (a0)\n", Cpu::M68040, false));
    assert!(!accepts(" pmove tt0,(a0)\n", Cpu::M68020, true));
    assert!(accepts(" pmove tt0,(a0)\n", Cpu::M68030, false));
}

#[test]
fn test_mmu_encoding_differs_by_cpu() -> Result<(), Box<dyn Error>> {
    let bytes = |cpu| write_bin(&ours(" pflusha\n", cpu, false));
    assert_eq!(bytes(Cpu::M68030)?, [0xF0, 0x00, 0x24, 0x00]);
    assert_eq!(bytes(Cpu::M68040)?, [0xF5, 0x18]);
    assert_eq!(bytes(Cpu::M68060)?, [0xF5, 0x18]);
    Ok(())
}

#[test]
fn test_mmu_names_are_ordinary_symbols_elsewhere() {
    let source = "tc: nop\ncrp: nop\n dc.l tc,crp\n";
    assert!(!ours(source, Cpu::M68030, false).has_errors());
}
