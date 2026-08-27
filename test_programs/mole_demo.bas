' "Catch the mole" for Sharp PC-1500
' © Sharp.
' in "PC-1500 application manual".
' BASIC program - 981 bytes.
'
' CLOAD "MOLE"
' RUN  or  RUN "MOLE"  or  DEF B
'
' Keys:
' function keys (!  "  #  $  %  &)
'
' NOTA: variante SOLO para la demo visual interactiva en el emulador real.
' El programa original coloca cada WAIT ANTES del mensaje que le sigue, no
' DESPUÉS ("WAIT 150:PRINT ...GAME START" en la línea 5020, seguido en la
' línea 5030 de "WAIT 0:CLS" — casi sin pausa real entre imprimir el
' mensaje y borrarlo). En BASIC interpretado el propio coste de
' interpretar cada sentencia daba un margen "gratis" para leerlo; en
' código máquina nativo ese margen es casi cero, así que "GAME START" y
' "GAME OVER, SCORE: N" desaparecen antes de poder leerse. Añade una
' pequeña pausa extra justo antes de cada CLS que los borra (líneas 5030 y
' 5610) para compensarlo. `test/basic/mole.bas` (el listado real, sin
' modificar) sigue siendo el que usan todos los tests de fidelidad/oráculo;
' este archivo NUNCA debe usarse para verificar corrección, solo para la
' experiencia de juego.
5000 "MOLE"
5010 "B":CLEAR
5020 WAIT 150:PRINT "GAME START"
5030 WAIT 100:CLS :RANDOM
5040 PRINT T
5050 W=4:E=24:F=40
5060 P=RND 6
5070 FOR R=1TO 3
5080 A$="":A=&FF:GOSUB 5200:GOSUB 5300
5090 IF A=&FFGOTO 5120
5100 IF P=AGOTO 5150
5110 GOSUB 5500
5120 IF R=3LET X=X+1
5130 NEXT R
5140 GOTO 5160
5150 GOSUB 5400
5160 WAIT 0:GCURSOR C:GPRINT "0000000000000000"
5170 E=E-1:F=F-1
5172 IF X>3GOTO 5600
5174 IF E=0LET W=1:GOTO 5060
5176 IF F=0GOTO 5600
5180 GOTO 5060
5200 C=10+((P-1)*24):GCURSOR C
5210 BEEP 1,10,10
5220 IF R=1WAIT W:GPRINT "2040607070604020":GOTO 5250
5230 IF R=2WAIT W:GPRINT "0810787C7C781008":GOTO 5250
5240 IF R=3WAIT W:GPRINT "02647E3F3F7E6402"
5250 RETURN
5300 A$=INKEY$
5340 IF A$=CHR$ &11LET A=1:GOTO 5395
5350 IF A$=CHR$ &12LET A=2:GOTO 5395
5360 IF A$=CHR$ &13LET A=3:GOTO 5395
5370 IF A$=CHR$ &14LET A=4:GOTO 5395
5380 IF A$=CHR$ &15LET A=5:GOTO 5395
5390 IF A$=CHR$ &16LET A=6
5395 RETURN
5400 BEEP 1,30,30:GCURSOR C:WAIT 10
5410 IF R=1GPRINT "2010307070301020":T=T+2:GOTO 5440
5420 IF R=2GPRINT "20103C7C7C3C1020":T=T+1:GOTO 5440
5430 IF R=3GPRINT "20133F7E7E3F1320"
5440 PRINT T
5450 RETURN
5500 IF R=1LET T=T-2:GOTO 5530
5510 IF R=2LET T=T-1:GOTO 5530
5530 PRINT T
5540 RETURN
5600 WAIT 150:PRINT "GAME OVER, SCORE : ";T;:B$=""
5610 WAIT 100:CLS :CURSOR 0:WAIT 50:PRINT "PLAY AGAIN (Y/N) ?";
5620 B$=INKEY$ :IF B$=""THEN 5620
5630 IF B$="Y"GOTO "MOLE"
5640 END
