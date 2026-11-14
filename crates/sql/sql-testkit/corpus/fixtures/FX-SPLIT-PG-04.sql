-- FX-SPLIT-PG-04  nested block comments and E'' escapes
/* outer /* inner ; */ still comment ; */
SELECT E'it\'s; fine' AS v;
SELECT 'plain;' AS w;
