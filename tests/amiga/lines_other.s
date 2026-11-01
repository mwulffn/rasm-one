; The second source for lines_main.s: one section joins its code, and the other is a hunk of
; its own.

	xdef	near_sub,far_sub,far_base

	section code,code_c
near_sub:
	moveq	#6,d7
	rts

	section far,code_c
far_base:
far_sub:
	moveq	#7,d7
	rts
