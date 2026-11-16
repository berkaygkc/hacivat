-- FX-SPLIT-MY-04  hash comments, backticks, versioned comments
SELECT `a;b` FROM (SELECT 1 AS `a;b`) t; # trailing ; comment
/*!80000 SELECT 'versioned;' */;
-- dash comment ;
SELECT 2;
