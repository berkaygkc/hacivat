CREATE TABLE loose (
    a int,
    CONSTRAINT loose_chk_2 CHECK ((`a` < 100))
);
