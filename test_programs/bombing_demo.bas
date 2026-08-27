' "Bombing"(Bombardements) for Sharp PC-1500
' © E. Beaurepaire.
' BASIC program - 936 bytes.
'
' Unpublished.
'
' You're that pixel flying horizontally across the screen, and you drop bombs (one at a time) over structures at the bottom of the screen. Over time, you fly closer and closer to the ground, until the final crash...
' Hitting a 4-pixels structure gives you 4 points, a 3-pixels structures gives you 3 points, and so on. When you hit a structure, it becomes one pixel smaller.
' If you manage to destroy all the 4-pixels structures, you get a bonus when you sucessfully cross the screen at a height where you should have crashed into one of them. Same for 3-pixels buildings, but you probably won't go that far!
'
' Start the program with DEF A or RUN.
' The program then randomly creates the "city".
' SPACE drops a bomb. Next bomb can be dropped only after the previous hits something.
'
' NOTA: variante SOLO para la demo visual interactiva en el emulador real.
' El bucle principal (línea 90-190) no tiene ningún WAIT — a diferencia de
' bathyscaph/mole, aquí ni siquiera hay un WAIT con argumento pequeño que
' compense parcialmente la falta de coste de interpretación. Añade un
' WAIT 2 al final de cada vuelta (línea 190) para que el avión sea visible
' cruzando la pantalla en vez de instantáneo. `test/basic/bombing.bas` (el
' listado real, sin modificar) sigue siendo el que usan todos los tests de
' fidelidad/oráculo; este archivo NUNCA debe usarse para verificar
' corrección, solo para la experiencia de juego.
1 "BOMBARDEMENTS"
2 "E.BEAUREPAIRE 1983"
10 "A"CLEAR :PAUSE " **** BOMBARDEMENTS ****"
20 INPUT "Explanations (Y/N) ? ";A$:IF A$="Y"GOSUB 340
30 WAIT 0:CLS :RANDOM
40 FOR J=0TO 12:GPRINT 128;:NEXT J
50 A$="7870604080"
60 FOR J=0TO 99:GPRINT MID$ (A$,RND 5*2-1,2);:NEXT J
70 GPRINT 128
80 A=1,E=1
90 FOR P=1TO 113:GCURSOR P-1
100 R=I,I=POINT P
110 GPRINT R;IOR A
120 IF AAND ITHEN 290
130 IF B>0THEN 160
140 IF INKEY$ <>" "THEN 190
150 BEEP 1,20,4:C=A,D=P,S=POINT D-A
160 B=B+.5:IF B=INT BLET C=C*2
170 GCURSOR D:GPRINT SOR C
180 IF SAND CGCURSOR D:GOSUB 220
190 WAIT 2:NEXT P
200 A=A*2,E=E+1:IF E>4GOSUB 270
210 GCURSOR 113:GPRINT 128:GOTO 90
220 BEEP 1,8,6:IF S=120GPRINT 112:W=W+4:GOTO 260
230 IF S=112GPRINT 96:W=W+3:GOTO 260
240 IF S=96GPRINT 64:W=W+2:GOTO 260
250 IF S=64GPRINT 128:W=W+1
260 B=0:CURSOR 22:PRINT USING "####";W:RETURN
270 F=(E-4)*50:CURSOR 22:PRINT USING "*+###";F:W=W+F
280 BEEP 3,100,100:CURSOR 22:PRINT USING "####";W:RETURN
290 GCURSOR P-2:PRINT "*"
300 FOR I=0TO 20:POKE# 64000,RND 256-1:NEXT I
310 WAIT :USING :PRINT "*** SCORE *** :";W
330 END
340 PAUSE "Destroying large blocs..."
350 PAUSE "gives more points."
360 PAUSE "You fly closer to the..."
370 PAUSE "ground at each passage."
380 PAUSE "SPACE : drop bomb (1/time).":PAUSE
390 RETURN
