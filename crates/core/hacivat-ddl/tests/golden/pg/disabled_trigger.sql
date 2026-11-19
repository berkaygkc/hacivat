CREATE TABLE paused (
    id integer NOT NULL
);

CREATE TRIGGER paused_audit AFTER UPDATE ON shop.paused FOR EACH ROW EXECUTE FUNCTION shop.audit_row();

ALTER TABLE paused DISABLE TRIGGER paused_audit;
