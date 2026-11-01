//! Compares rasm-one with vasm -no-opt byte for byte.

use std::error::Error;

use rasm_one::{assemble, write_bin, Assembly, MemoryProvider, Options};
use rasm_oracle::{Format, Vasm};

fn ours(source: &str) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    assemble(&Options::new("t.s"), &files)
}

/// Sources both assemblers must accept, with identical bytes.
const ACCEPTED: &[&str] = &[
    // Labels, comments and field rules.
    "lab nop",
    "lab: nop",
    " lab: nop",
    "lab:: nop",
    "*comment\n nop",
    " nop ;c",
    " nop *c",
    " nop d0",
    " nop nop",
    "nop",
    " NOP",
    " MOVE.L D0,A0",
    " move.l d0,d1 it's a comment",
    " dc.b 1, 2",
    " dc.b 1 ,2",
    " dc.w 1 + 2",
    " dc.w 1 *2",
    " dc.w 1*2;x",
    // Expressions.
    " dc.l 1+2<<3",
    " dc.l 2*3|1",
    " dc.l 6&3+1",
    " dc.l 1<2,2<1,1<=1,1>=2,1>2,1=1,1==1,1!=1,1<>2",
    " dc.l !0,!1,~0,-1,+1",
    " dc.l 7!8,7^2,7~2,1&&2,1||0,0&&1,0||0",
    " dc.l 7/2,-7/2,-7%3",
    " dc.l %101,@17,$fF,10",
    " dc.l (1+2)*3",
    // Data and alignment.
    " dc.b 1\nl: dc.w l",
    " dc.b 1\nl:\n nop\n dc.l l",
    " dc.b 1\nl: nop\n dc.l l",
    " dc.b 1\nl: dc.l l",
    " dc.b 1\n nop",
    " dc.b 1\n ds.w 1",
    " dc.b 1\n blk.l 1,2",
    "l: dc.b 1\n dc.b 2\nm: ds.b 1\n dc.w m",
    " dc.b 255,-128",
    " dc.w 65535,-32768",
    " dc.l $ffffffff,-$80000000",
    " dc.l \"ab\"",
    " dc.b 'a',\"bc\"",
    " dc.w 'ab'",
    " dc.b \"a\"\"b\"",
    " dc.b \"\\t\",\"\"",
    " dc.b 'a'+1",
    " dc 1",
    " ds 1",
    " blk 1",
    " blk.b 2",
    " blk.w 2,$10000",
    " blk.b 2,$1ff",
    " ds.b g-f\nf: nop\ng:",
    " dc.l *,*+2",
    " dc.w f+1,f-g,g-f,g+1\nf: nop\ng:",
    " dc.b f\nf:",
    " move.w f(a0),d0\nf:",
    " move.w f(a0,d0.w),d0\nf:",
    " move.w #*,d0",
    " move.l #*,4(a0)",
    " move.w d0,*(a0)",
    " move.w 2(a0),*+2(a1)",
    // Immediates.
    " move.b #-1,d0",
    " move.b #255,d0",
    " move.w #$ffff,d0",
    " move.w #-32768,d0",
    " move.l #$ffffffff,d0",
    " move.b #f,d0\nf:",
    " move.w #f,d0\nf:",
    " move.l #f,d0\nf:",
    " move.l #\"ab\",d0",
    // Absolute addresses.
    " move.w $7fff.w,d0",
    " move.w $ffff8000.w,d0",
    " move.w 4,d0",
    " move.w (4).w,d0",
    " move.w (4),d0",
    " move.w (4).l,d0",
    " move.w f.w,d0\nf:",
    " move.w f.l,d0\nf:",
    // Displacements and indexes.
    " move.w $ffff8000(a0),d0",
    " move.w $ffffff80(a0,d0.w),d0",
    " move.w (4+2)*3(a0),d0",
    " move.w (4)(a0),d0",
    " move.w 2+(4)(a0),d0",
    " move.w -(4)(a0),d0",
    " move.w (4,a0),d0",
    " move.w (4,a0,d0.l),d0",
    " move.w (a0,d0),d0",
    " move.w (a0,a1.l),d0",
    " move.w (a0),d0",
    " move.w (sp)+,sp",
    " move.w d0,-(a7)",
    " move.w -(sp),(a0)+",
    // PC-relative.
    " move.w 4(pc),d0",
    " lea 4(pc),a0",
    "f: lea f(pc),a0",
    " lea *+2(pc),a0",
    " move.w (4,pc),d0",
    " move.w (4,pc,d1.w),d0",
    " move.w 4(pc,d1),d0",
    " move.w (pc),d0",
    " move.w (pc,d1),d0",
    // Sizes and row choice.
    " move d0,d1",
    " movea d0,a1",
    " movea.l (a0)+,a1",
    " lea.l (a0),a0",
    " move.w a0,d0",
    " move.l a0,a1",
    " move.l #1,4(a0,d1.l)",
    " move.l $4.w,$8.w",
    // Branches.
    "l: bra.s l",
    " bra.b l\n nop\nl:",
    " bra.w l\nl:",
    " bra l\nl:",
    " bra 4",
    " bhs *",
    " blo *",
    " bsr.w *+1",
    "l: bhi l\n bls l\n bcc l\n bcs l\n bne l\n beq l\n bvc l\n bvs l",
    "l: bpl l\n bmi l\n bge l\n blt l\n bgt l\n ble l\n bsr l",
    // Sections.
    " section a,bss\n ds.w 1",
    " section a,code\n section b,data\n nop",
    " section a",
    " section a,code_c\n nop",
    " section a,data_f,chip\n nop",
    " section a,bss\n nop",
    " section a,bss\n dc.w 1",
    " SECTION a,CODE\n LEA (A0),SP",
    // A small program.
    "    section code,code\n\
     start:\n\
     \tmove.l  #$12345678,d0 ; load\n\
     \tlea     data(pc),a0\n\
     \tmove.w  (a0),d1\n\
     \tbsr.s   sub\n\
     \tbeq     done\n\
     \tmove.l  table,a1\n\
     \tmove.w  table+2(pc),4(a1,d1.w)\n\
     done:\trts\n\
     sub:\tmove.w  d1,d0\n\
     \trts\n\
     * data follows\n\
     table:\tdc.l    start,done\n\
     data:\tdc.w    $BEEF,table-start\n\
     text\tdc.b    \"hi\",0\n\
     \tblk.w   2,$ff\n",
];

/// Valid syntax that both assemblers must reject.
const REJECTED: &[&str] = &[
    " move.l d0, d1",
    " move.l d0 ,d1",
    " dc.w 1+ 2",
    " dc.b 256",
    " dc.b -129",
    " dc.w 65536",
    " dc.w -32769",
    " dc.w 'abc'",
    " dc.b f-1000\nf:",
    " dc.l",
    " dc.b ,",
    " dc.b 1,",
    " dc.l 1/0",
    " dc.l 1%0",
    " move.b #256,d0",
    " move.b #-129,d0",
    " move.w #65536,d0",
    " move.w #-32769,d0",
    " move.l #$100000000,d0",
    " move.w $8000.w,d0",
    " move.w $ffff.w,d0",
    " move.w $10000.w,d0",
    " move.w -32769.w,d0",
    " move.w 40000(a0),d0",
    " move.w $ffff(a0),d0",
    " move.w 128(a0,d0.w),d0",
    " move.w 255(a0,d0.w),d0",
    " move.w -129(a0,d0.w),d0",
    " move.w 4(a0,d0.b),d0",
    " bra.s l\nl:",
    " bsr.s *+200",
    " bra.l l\nl:",
    " bra d0",
    " bra (a0)",
    " bra #4",
    " move.b d0,a0",
    " move.b a0,d0",
    " move.w d0,#1",
    " move.w d0,4(pc)",
    " movea.b d0,a1",
    " move.q d0,d1",
    " lea d0,a0",
    " lea (a0)+,a0",
    " lea.w (a0),a0",
    " lea ,a0",
    " move.w",
    " move.w d0",
    " move.w d0,d1,d2",
    " move.w d0,",
    "1abc: nop",
    " lab nop",
    "foo: nop\nfoo: nop",
    " dc.l undefined",
    " bogus",
    " section a,foo",
    " section a,code\n nop\n section b,data\n dc.w 1",
];

/// Deliberately stricter than vasm, which only warns or misreads these. The
/// brief asks for a strict dialect in which anything else is a clear error.
const STRICTER_THAN_VASM: &[&str] = &[
    " move.w d0.w,d1",
    " move.w (a0).w,d1",
    " move.w 4.w(a0),d0",
    " dc.l 0.w",
    " dc.b \"a",
    " dc.b (1",
    " dc.b 1)",
    " ds.l 1,2",
    " ds.b l\nl:",
    "a0: nop",
    "sp: nop",
    " dc d0",
];

#[test]
fn test_assemble_accepted_sources_match_vasm_bytes() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in ACCEPTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68000", Format::Bin)?;
        assert!(
            theirs.success,
            "vasm rejected {source:?}: {}",
            theirs.stderr
        );
        let assembly = ours(&source);
        match write_bin(&assembly) {
            Ok(bytes) if bytes == theirs.bytes => {}
            Ok(bytes) => failures.push(format!(
                "{source:?}\n  ours  {bytes:02x?}\n  vasm  {:02x?}",
                theirs.bytes
            )),
            Err(error) => failures.push(format!("{source:?}: {error}: {:?}", assembly.diagnostics)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_assemble_rejected_sources_match_vasm_rejection() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in REJECTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68000", Format::Bin)?;
        assert!(!theirs.success, "vasm accepted {source:?}");
        if write_bin(&ours(&source)).is_ok() {
            failures.push(source);
        }
    }
    assert!(failures.is_empty(), "we accepted {failures:?}");
    Ok(())
}

#[test]
fn test_assemble_stricter_than_vasm_sources_are_errors() {
    for source in STRICTER_THAN_VASM {
        let source = format!("{source}\n");
        assert!(ours(&source).has_errors(), "{source:?}");
    }
}
