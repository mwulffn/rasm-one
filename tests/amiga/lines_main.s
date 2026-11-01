; Checks line debug information on a real loader, with scripts/emulator-lines.py. Each line
; with "moveq #N,d7" is a line it expects to stop on, and N says which. Waits at the start, so
; that the script can set breakpoints once AmigaDOS has placed the hunks.

	xref	near_sub,far_sub,far_base

twice	macro
	moveq	#4,d7
	moveq	#5,d7
	endm

	section code,code_c
start:
	lea	start(pc),a0
	move.l	a0,$100.w
	lea	far_base,a0
	move.l	a0,$108.w
	clr.l	$10c.w
	clr.l	$110.w
	move.l	#"RASM",$104.w
.wait:	tst.l	$10c.w
	beq.s	.wait

	moveq	#1,d7
	include	"lines_inc.i"
	twice
	twice
	jsr	near_sub
	jsr	far_sub
	moveq	#8,d7
	move.l	#"DONE",$110.w
.stay:	bra.s	.stay
