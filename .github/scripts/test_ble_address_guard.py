import tempfile
from pathlib import Path
import unittest

from ble_address_guard import check


class AddressGuardTest(unittest.TestCase):
    def fixture(self, code, row="firmware/main/example.c:advertise"):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        (root / 'firmware/main').mkdir(parents=True)
        (root / 'schema/protocol').mkdir(parents=True)
        (root / 'firmware/main/example.c').write_text(code)
        (root / 'schema/protocol/net-v2.md').write_text(
            '**Emitters**\n\n| Emitter |\n|---|\n| `' + row + '` |\n\nOther text')
        return root

    def test_private_documented_emitter_passes(self):
        self.assertEqual(check(self.fixture('void advertise(void) {\nble_hs_id_infer_auto( 1, &type);\nble_gap_adv_start(private_type);\n}')), [])

    def test_public_selection_and_literal_fail(self):
        for code in ('ble_hs_id_infer_auto(0, &type);', 'ble_hs_id_infer_auto(false, &type);',
                     'ble_gap_adv_start(BLE_OWN_ADDR_PUBLIC);', 'ble_gap_adv_start(0, args);'):
            self.assertTrue(check(self.fixture('void advertise(void) {\n' + code + '\n}')))

    def test_new_emitter_needs_inventory_entry(self):
        for call in ('ble_gap_adv_start', 'ble_gap_ext_adv_start', 'esp_ble_gap_start_advertising'):
            self.assertTrue(check(self.fixture('void unknown(void) {\n' + call + '(private_type);\n}')))

    def test_comments_do_not_create_or_hide_calls(self):
        self.assertEqual(check(self.fixture('/* ble_gap_adv_start(BLE_OWN_ADDR_PUBLIC); */')), [])
        self.assertTrue(check(self.fixture('void advertise(void) {\nble_hs_id_infer_auto(/* privacy */ 0, &type);\n}')))
