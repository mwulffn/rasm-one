; Run by AmigaDOS from a floppy, so the system's own loader has to accept the
; hunks, place them and apply the relocations. Follows a pointer across hunks,
; so a wrong relocation or section gives a wrong result or a crash. Leaves the
; result's address at $100, then "RASM" at $104 to say it is done, and stays put.
; Memory holds leftovers at boot, so scripts/emulator-smoke.sh waits for the mark.

	section code,code_c
start:
	lea	message(pc),a0		; PC-relative within the hunk
	move.l	(a0),d1
	move.l	pointer,a2		; absolute, relocated to the data hunk
	move.l	(a2),d0			; the data hunk's pointer, relocated to itself
	add.l	d1,d0
	lea	result,a1		; absolute, relocated to the BSS hunk
	move.l	d0,(a1)
	move.l	a1,$100.w
	move.l	#"RASM",$104.w
.stay:	bra.s	.stay
message:
	dc.l	$1000

	section numbers,data
pointer:
	dc.l	value
value:
	dc.l	$C0DE

	section space,bss
result:
	ds.l	1
