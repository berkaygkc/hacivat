CREATE TABLE audit (
    at datetime
);

CREATE /*!50017 DEFINER=`root`@`%`*/ TRIGGER `audit_bi` BEFORE INSERT ON `audit` FOR EACH ROW SET NEW.at = NOW();
