CREATE TABLE loose (
    a int,
    CONSTRAINT loose_chk_2 CHECK ((`a` < 100))
);

ALTER TABLE loose ADD CONSTRAINT loose_chk_1 CHECK ((`a` > 0)) NOT ENFORCED;
