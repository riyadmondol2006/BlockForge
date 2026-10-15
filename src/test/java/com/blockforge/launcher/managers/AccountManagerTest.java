/*
 * BlockForge - a fork of ATLauncher (https://github.com/ATLauncher/ATLauncher)
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program. If not, see <http://www.gnu.org/licenses/>.
 */
package com.blockforge.launcher.managers;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Arrays;
import java.util.Collections;

import org.junit.jupiter.api.Test;

import com.blockforge.launcher.data.MicrosoftAccount;

class AccountManagerTest {
    @Test
    void removingTheSelectedFirstAccountSwitchesToAnotherOne() {
        MicrosoftAccount first = MicrosoftAccount.createOffline("Alex");
        MicrosoftAccount second = MicrosoftAccount.createOffline("Steve");

        assertSame(second, AccountManager.accountToSelectAfterRemoving(Arrays.asList(first, second), first));
        assertSame(first, AccountManager.accountToSelectAfterRemoving(Arrays.asList(first, second), second));
        assertNull(AccountManager.accountToSelectAfterRemoving(Collections.singletonList(first), first));
    }

    @Test
    void offlineNamesFollowMinecraftsRules() {
        assertTrue(AccountManager.isValidOfflineUsername("Steve"));
        assertTrue(AccountManager.isValidOfflineUsername("abc"));
        assertTrue(AccountManager.isValidOfflineUsername("a_very_long_name"));

        assertFalse(AccountManager.isValidOfflineUsername(null));
        assertFalse(AccountManager.isValidOfflineUsername("ab"));
        assertFalse(AccountManager.isValidOfflineUsername("a_name_that_is_17"));
        assertFalse(AccountManager.isValidOfflineUsername("two words"));
        assertFalse(AccountManager.isValidOfflineUsername("Zoë"));
        assertFalse(AccountManager.isValidOfflineUsername(" Steve"));
    }
}
