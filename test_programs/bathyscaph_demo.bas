' "Bathyscaph" for Sharp PC-1500 or PC-1500A.
' © L'Ordinateur Individuel.
' Author : Philippe Caïric
' Published in "L'Ordinateur Individuel" #48 (May 1983).
' BASIC program - 907 bytes.
'
' CLOAD "BATHYSCAPH"
' RUN or DEF Z
'
' Game: Pilot your submarine through the cave and reach the exit as fast as possible. If you crash, you start again and your time adds up.
'
' Keys:
' cursor up - up
' cursor down - down
'
' NOTA: variante SOLO para la demo visual interactiva en el emulador real.
' Añade un `WAIT` al bucle P (línea 110) para compensar que el código
' máquina nativo no tiene el coste de interpretación de bytecode que hacía
' de "regulador de velocidad" implícito en el programa original — sin él,
' el bucle corre miles de veces más rápido de lo jugable. `test/basic/
' bathyscaph.bas` (el listado real de 1983, sin modificar) sigue siendo el
' que usan todos los tests de fidelidad/oráculo; este archivo NUNCA debe
' usarse para verificar corrección, solo para la experiencia de juego.
1 "BATHYSCAPH"
2 "Move : up/down arrows
8 "Z"CLS :WAIT 0:CLEAR :DIM A$(0)*20:GCURSOR 10
10 FOR I=0TO 13
20 RESTORE 999+RND 16
30 READ A$(0):GPRINT A$(0);
40 NEXT I:GCURSOR 154:GPRINT "141C":TIME =0,H=3,G=8
50 FOR P=2TO 154:GCURSOR P-2
60 Z$=INKEY$ :IF Z$=""THEN 85
70 H=H-SGN (ASC Z$-10.5)
80 G=INT (2^H+.5)
85 S=R:R=Q:Q=POINT P
90 GPRINT S;GOR R;GOR Q
100 IF (QAND G)>0GOSUB "CRASH"
110 WAIT 3:BEEP 1,0,1:NEXT P:IF H<>3THEN 50
120 CURSOR 0:PRINT "TIME :";TIME *100;" mn ":FOR I=1TO 5:BEEP 1,105,80:BEEP 1,82,80:BEEP 1,68,80:NEXT I
130 END
200 "CRASH"FOR I=0TO 30:POKE# 64000,RND 256-1:NEXT I
210 CLEAR :P=2,G=8,H=3:RETURN
1000 DATA "7163470F1F0F47637160"
1001 DATA "717371787C7E78606141"
1002 DATA "776341085D5D08416371"
1003 DATA "00404040606070707060"
1004 DATA "00406070787C7C704040"
1005 DATA "0060787E7C7870604000"
1006 DATA "70707070604078706070"
1007 DATA "61634341406071737361"
1008 DATA "0103070F0703416070"
1009 DATA "07070303416170787060"
1010 DATA "07600760406070794060"
1011 DATA "41414141181841414141"
1012 DATA "41636363616041430341"
1013 DATA "01034347676763434101"
1014 DATA "18181818414143431840"
1015 DATA "00010301000103070361"
