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
package com.blockforge.launcher.content;

import java.lang.reflect.InvocationTargetException;

import javax.swing.SwingUtilities;

import com.blockforge.launcher.gui.dialogs.ProgressDialog;

/**
 * The launcher's download code reports its progress to a progress dialog. This one is never shown: it passes the
 * numbers on, so progress can be drawn right where the player clicked (on a card, a row or a button).
 */
final class SilentProgress extends ProgressDialog<Void> {
    private static final long serialVersionUID = 1L;

    private final transient ContentService.Progress target;
    private double total;
    private double done;

    private SilentProgress(ContentService.Progress target) {
        super("", false, null);
        this.target = target;
    }

    /** Swing windows have to be made on the interface thread, even ones that are never shown. */
    static SilentProgress create(ContentService.Progress target) {
        if (SwingUtilities.isEventDispatchThread()) {
            return new SilentProgress(target);
        }
        final SilentProgress[] made = new SilentProgress[1];
        try {
            SwingUtilities.invokeAndWait(() -> made[0] = new SilentProgress(target));
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IllegalStateException(e);
        } catch (InvocationTargetException e) {
            throw new IllegalStateException(e.getCause());
        }
        return made[0];
    }

    private void report() {
        target.fraction(total > 0 ? Math.min(1.0, done / total) : -1);
    }

    @Override
    public void setTotalBytes(long bytes) {
        total = bytes;
        done = 0;
        report();
    }

    @Override
    public void addDownloadedBytes(long bytes) {
        done += bytes;
        report();
    }

    @Override
    public void addBytesToDownload(long bytes) {
        total += bytes;
        report();
    }

    @Override
    public void clearDownloadedBytes() {
        done = 0;
        report();
    }

    @Override
    public void setLabel(String text) {
        target.status(text);
    }

    @Override
    public void setSubProgress(double percent, String label) {
        target.fraction(percent < 0 ? -1 : percent / 100.0);
    }

    @Override
    public void setIndeterminate() {
        target.fraction(-1);
    }

    @Override
    public void doneTask() {
        // nothing to show
    }

    @Override
    public void start() {
        // never shown
    }

    @Override
    public void close() {
        if (SwingUtilities.isEventDispatchThread()) {
            dispose();
        } else {
            SwingUtilities.invokeLater(this::dispose);
        }
    }
}
