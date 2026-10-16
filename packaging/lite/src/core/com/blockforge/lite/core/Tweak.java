package com.blockforge.lite.core;

/**
 * A change that is not a class patch: something the agent does at run time (for example adjusting threads). It has an
 * id, a level and is switched with off=/only= like a patch, and its decision goes into the report and the summary.
 * It never matches a class; {@link #start} is called once when the agent starts, if the tweak is on.
 */
abstract class Tweak extends Patch {
    Tweak(String id, int level, String what, String[] conflictsWith) {
        super(id, level, what, new String[0][], conflictsWith);
    }

    @Override
    final void apply(Target t, Ctx ctx) throws Skip {
        throw Skip.notTarget("a tweak does not change classes");
    }

    /**
     * Starts the tweak (it must return quickly; run work on a daemon thread) and returns null when it is on, or why it
     * is not ("4 or more processors").
     */
    abstract String start();
}
