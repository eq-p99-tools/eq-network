use super::*;

const NORMAL: Band = Band {
    beneficial: false,
    short: false,
    last: false,
};
const BENEFICIAL: Band = Band {
    beneficial: true,
    short: false,
    last: false,
};
const SONG: Band = Band {
    beneficial: true,
    short: true,
    last: false,
};

/// A spell with these band fields and these leading effects
/// (id, base, formula), the rest blank, and no Bard level.
fn spell(id: u32, band: Band, effects: &[(u32, i32, u32)]) -> Spell {
    let mut slots = [Effect::default(); 12];
    for (slot, (effect, base, formula)) in slots.iter_mut().zip(effects) {
        *slot = Effect {
            id: *effect,
            base: *base,
            formula: *formula,
        };
    }
    Spell {
        id,
        band: Some(band),
        bard: Some(false),
        effects: Some(slots),
    }
}

/// A plain spell with one unrelated effect, which conflicts with nothing
/// here.
fn plain(id: u32, band: Band) -> Spell {
    spell(id, band, &[(0x0d, 1, 100)])
}

fn held(slots: &[(u32, Spell)]) -> BTreeMap<u32, Spell> {
    slots.iter().copied().collect()
}

/// Fills slots with plain spells numbered from 1000.
fn filled(slots: Range<u32>, band: Band) -> Vec<(u32, Spell)> {
    slots.map(|slot| (slot, plain(slot + 1000, band))).collect()
}

fn place(held: &BTreeMap<u32, Spell>, spell: Spell) -> Result<Placed, Unplaced> {
    titanium_player(&Landing {
        held,
        spell,
        blocker: false,
        others_add_no_slots: false,
    })
}

fn slot(held: &BTreeMap<u32, Spell>, spell: Spell) -> Result<u32, Unplaced> {
    place(held, spell).map(|placed| placed.slot)
}

#[test]
fn a_new_buff_takes_the_first_empty_normal_slot() {
    let new = plain(278, NORMAL);
    assert_eq!(slot(&held(&[]), new), Ok(0));
    // A faded buff's slot is the next landing's, ahead of later ones.
    let two = held(&[(1, plain(10, NORMAL)), (2, plain(11, NORMAL))]);
    assert_eq!(slot(&two, new), Ok(0));
    let gap = held(&[(0, plain(9, NORMAL)), (2, plain(11, NORMAL))]);
    assert_eq!(slot(&gap, new), Ok(1));
    // Songs held do not take normal slots.
    assert_eq!(slot(&held(&[(25, plain(700, SONG))]), new), Ok(0));
}

#[test]
fn a_held_spell_keeps_its_first_slot() {
    let slots = held(&[
        (0, plain(9, NORMAL)),
        (3, plain(278, NORMAL)),
        (26, plain(278, NORMAL)),
    ]);
    assert_eq!(place(&slots, plain(278, NORMAL)), Ok(Placed::at(3)));
    // Even with no spell data: the step needs none.
    assert_eq!(slot(&slots, Spell::id(278)), Ok(3));
    // Or under a blocker: a refusal writes nothing, so the record stays.
    let blocked = |spell| {
        titanium_player(&Landing {
            held: &slots,
            spell,
            blocker: true,
            others_add_no_slots: false,
        })
    };
    assert_eq!(blocked(plain(278, NORMAL)).map(|placed| placed.slot), Ok(3));
    assert_eq!(blocked(plain(279, NORMAL)), Err(Unplaced::Blocked));
}

#[test]
fn songs_take_the_song_band_then_fall_back_to_the_normal_one() {
    let song = plain(700, SONG);
    assert_eq!(slot(&held(&[]), song), Ok(25));
    let some = held(&[(25, plain(701, SONG)), (0, plain(9, NORMAL))]);
    assert_eq!(slot(&some, song), Ok(26));
    let full = held(&filled(TITANIUM_SONG_SLOTS, SONG));
    assert_eq!(slot(&full, song), Ok(0));
}

#[test]
fn the_last_slot_has_its_own_band_and_each_band_needs_both_its_flags() {
    let last = Band {
        beneficial: true,
        short: false,
        last: true,
    };
    assert_eq!(slot(&held(&[]), plain(800, last)), Ok(37));
    assert_eq!(
        slot(&held(&[(37, plain(801, last))]), plain(800, last)),
        Err(Unplaced::BandFull)
    );
    // +0x17d alone, or +0x19b or +0x1c4 without it: the normal band.
    for band in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let band = Band {
            beneficial: band.0,
            short: band.1,
            last: band.2,
        };
        assert_eq!(slot(&held(&[]), plain(800, band)), Ok(0));
    }
}

#[test]
fn missing_facts_leave_a_landing_unslotted() {
    assert_eq!(
        slot(&held(&[]), Spell::id(278)),
        Err(Unplaced::SpellFieldsUnknown)
    );
    // A held spell whose effects are unknown could be one step 4 replaces.
    let unknown = held(&[(0, Spell::id(9))]);
    assert_eq!(
        slot(&unknown, plain(278, NORMAL)),
        Err(Unplaced::SpellFieldsUnknown)
    );
}

#[test]
fn the_normal_band_names_slots_below_what_held_buffs_add() {
    let fifteen = held(&filled(0..15, NORMAL));
    // The worn items and abilities may add more: unknown.
    assert_eq!(
        slot(&fifteen, plain(278, NORMAL)),
        Err(Unplaced::TopUnknown)
    );
    // A held buff adding 3 slots names slots 15 to 17.
    let mut adding = filled(0..15, NORMAL);
    adding[0].1 = spell(1000, NORMAL, &[(ADDS_SLOTS, 3, 100)]);
    assert_eq!(slot(&held(&adding), plain(278, NORMAL)), Ok(15));
    adding.extend(filled(15..18, NORMAL));
    assert_eq!(
        slot(&held(&adding), plain(278, NORMAL)),
        Err(Unplaced::TopUnknown)
    );
    // A value that can't be worked out adds nothing named.
    let mut scaled = filled(0..15, NORMAL);
    scaled[0].1 = spell(1000, NORMAL, &[(ADDS_SLOTS, 3, 102)]);
    assert_eq!(
        slot(&held(&scaled), plain(278, NORMAL)),
        Err(Unplaced::TopUnknown)
    );
    // Ten or more is the most: the band is 25 and its top is known.
    let mut most = filled(0..25, NORMAL);
    most[0].1 = spell(1000, NORMAL, &[(ADDS_SLOTS, 12, 100)]);
    assert_eq!(
        slot(&held(&most), plain(278, NORMAL)),
        Err(Unplaced::BandFull)
    );
    // With slots 0 to 24 full, the top doesn't matter either.
    let all = held(&filled(0..25, NORMAL));
    assert_eq!(slot(&all, plain(278, NORMAL)), Err(Unplaced::BandFull));
    // Only a caller that knows the other sources add nothing fixes it at 15.
    let known = titanium_player(&Landing {
        held: &fifteen,
        spell: plain(278, NORMAL),
        blocker: false,
        others_add_no_slots: true,
    });
    assert_eq!(known, Err(Unplaced::BandFull));
}

#[test]
fn a_full_band_evicts_the_first_flagged_spell_for_an_unflagged_one() {
    let mut slots = filled(0..25, NORMAL);
    slots[4].1 = plain(1004, BENEFICIAL);
    slots[9].1 = plain(1009, BENEFICIAL);
    let slots = held(&slots);
    assert_eq!(
        place(&slots, plain(278, NORMAL)),
        Ok(Placed {
            slot: 4,
            removed: Some(4),
            rule: Some(Rule::Evicted),
        })
    );
    // A flagged landing evicts nothing.
    assert_eq!(
        slot(&slots, plain(279, BENEFICIAL)),
        Err(Unplaced::BandFull)
    );
    // A song held in its band is evicted, and the landing takes it: no
    // normal slot is empty [I].
    let mut songs = filled(0..25, NORMAL);
    songs.push((25, plain(700, SONG)));
    assert_eq!(
        place(&held(&songs), plain(278, NORMAL)),
        Ok(Placed {
            slot: 25,
            removed: Some(25),
            rule: Some(Rule::Evicted),
        })
    );
    // A held slot whose flag is unknown, before any flagged one, stops it.
    let mut unknown = filled(0..25, NORMAL);
    unknown[0].1.band = None;
    unknown[3].1 = plain(1003, BENEFICIAL);
    assert_eq!(
        slot(&held(&unknown), plain(278, NORMAL)),
        Err(Unplaced::SpellFieldsUnknown)
    );
}

/// Step 4 for one held spell in slot 2 (slots 0 and 1 hold plain ones).
fn over(held_spell: Spell, new: Spell) -> Result<Placed, Unplaced> {
    let slots = held(&[
        (0, plain(9, NORMAL)),
        (1, plain(10, NORMAL)),
        (2, held_spell),
    ]);
    place(&slots, new)
}

const fn replaced(slot: u32) -> Placed {
    Placed {
        slot,
        removed: Some(slot),
        rule: Some(Rule::Replaced),
    }
}

#[test]
fn a_stronger_value_replaces_in_place_and_a_weaker_is_refused() {
    let armor = |id, base| spell(id, NORMAL, &[(0x0d, 1, 100), (1, base, 100)]);
    assert_eq!(over(armor(400, 20), armor(401, 30)), Ok(replaced(2)));
    assert_eq!(over(armor(400, 20), armor(401, 20)), Ok(replaced(2)));
    assert_eq!(
        over(armor(400, 20), armor(401, 10)),
        Err(Unplaced::Refused(Rule::Weaker))
    );
    // Negative values: the more negative replaces.
    let debuff = |id, base| spell(id, NORMAL, &[(2, base, 0)]);
    assert_eq!(over(debuff(400, -20), debuff(401, -30)), Ok(replaced(2)));
    assert_eq!(
        over(debuff(400, -20), debuff(401, -10)),
        Err(Unplaced::Refused(Rule::Weaker))
    );
    // A zero held value refuses, but for the one spell let through.
    assert_eq!(
        over(debuff(400, 0), debuff(401, 5)),
        Err(Unplaced::Refused(Rule::Weaker))
    );
    assert_eq!(
        over(debuff(400, 0), debuff(LET_THROUGH, 5)),
        Ok(replaced(2))
    );
    // A formula whose evaluation is unread leaves it unslotted.
    let scaled = spell(401, NORMAL, &[(2, 30, 102)]);
    assert_eq!(over(debuff(400, 20), scaled), Err(Unplaced::ValueUnknown));
}

#[test]
fn effects_that_differ_never_conflict_nor_do_listed_or_blank_ones() {
    let new = spell(401, NORMAL, &[(2, 5, 100)]);
    assert_eq!(
        slot(&held(&[(0, spell(400, NORMAL, &[(3, 50, 100)]))]), new),
        Ok(1)
    );
    // Listed effects stack with themselves.
    let listed = |id| spell(id, NORMAL, &[(0x7c, 5, 100)]);
    assert_eq!(
        over(listed(400), listed(401)).map(|placed| placed.slot),
        Ok(3)
    );
    // A blank on either side ends the compare before later positions.
    let late = |id, first| spell(id, NORMAL, &[(first, 0, 100), (2, 5, 100)]);
    assert_eq!(
        over(late(400, BLANK), late(401, 3)).map(|placed| placed.slot),
        Ok(3)
    );
    // Effect 10 with a zero base on either side.
    let ten = |id, base| spell(id, NORMAL, &[(10, base, 100)]);
    assert_eq!(
        over(ten(400, 0), ten(401, 9)).map(|placed| placed.slot),
        Ok(3)
    );
    // Effect 1 with a negative new base.
    let one = |id, base| spell(id, NORMAL, &[(1, base, 100)]);
    assert_eq!(
        over(one(400, 5), one(401, -9)).map(|placed| placed.slot),
        Ok(3)
    );
}

#[test]
fn the_outright_refusals_come_before_the_values() {
    let effect = |id, effect| spell(id, NORMAL, &[(effect, 5, 100)]);
    assert_eq!(
        over(effect(400, 7), effect(401, 0x2c)),
        Err(Unplaced::Refused(Rule::NewEffectRefused))
    );
    assert_eq!(
        over(effect(400, 0x65), effect(401, 0x65)),
        Err(Unplaced::Refused(Rule::HeldKept))
    );
    assert_eq!(
        over(effect(0x4b0, 7), effect(401, 7)),
        Err(Unplaced::Refused(Rule::HeldKept))
    );
}

#[test]
fn opposite_signs_replace_but_for_effects_3_and_0() {
    let effect = |id, effect, base| spell(id, NORMAL, &[(effect, base, 100)]);
    assert_eq!(over(effect(400, 7, -5), effect(401, 7, 5)), Ok(replaced(2)));
    assert_eq!(over(effect(400, 7, 5), effect(401, 7, -5)), Ok(replaced(2)));
    assert_eq!(
        over(effect(400, 3, -5), effect(401, 3, 5)).map(|placed| placed.slot),
        Ok(3)
    );
    assert_eq!(
        over(effect(400, 0, -5), effect(401, 0, 5)),
        Err(Unplaced::Refused(Rule::Weaker))
    );
    assert_eq!(over(effect(400, 0, 5), effect(401, 0, -5)), Ok(replaced(2)));
    // The listed spells count as -1.
    assert_eq!(
        over(effect(400, 7, 5), effect(0x654, 7, 5)),
        Ok(replaced(2))
    );
}

#[test]
fn a_slow_replaces_a_haste_and_a_weaker_slow_but_not_the_reverse() {
    let speed = |id, base| spell(id, NORMAL, &[(11, base, 100)]);
    // A slow at least as strong replaces a slow, and any haste.
    assert_eq!(over(speed(400, 80), speed(401, 70)), Ok(replaced(2)));
    assert_eq!(over(speed(400, 130), speed(401, 70)), Ok(replaced(2)));
    // A haste can't replace a slow, nor a weaker slow a stronger one.
    assert_eq!(
        over(speed(400, 70), speed(401, 130)),
        Err(Unplaced::Refused(Rule::Weaker))
    );
    assert_eq!(
        over(speed(400, 70), speed(401, 80)),
        Err(Unplaced::Refused(Rule::Weaker))
    );
    assert_eq!(
        over(speed(400, 70), speed(LET_THROUGH, 80)),
        Ok(replaced(2))
    );
    // Haste over haste: the stronger or equal replaces.
    assert_eq!(over(speed(400, 130), speed(401, 140)), Ok(replaced(2)));
    assert_eq!(
        over(speed(400, 140), speed(401, 130)),
        Err(Unplaced::Refused(Rule::Weaker))
    );
}

#[test]
fn the_flag_rules_and_the_bard_exception() {
    let effect = |id, band, effect| spell(id, band, &[(effect, 5, 100)]);
    // A flagged landing over an unflagged hold refuses, but at effect 3.
    assert_eq!(
        over(effect(400, NORMAL, 7), effect(401, BENEFICIAL, 7)),
        Err(Unplaced::Refused(Rule::OverUnflagged))
    );
    // Both flagged, with the new one carrying effect 0x3a anywhere.
    let pair = spell(401, BENEFICIAL, &[(7, 5, 100), (0x3a, 1, 100)]);
    assert_eq!(
        over(effect(400, BENEFICIAL, 7), pair),
        Err(Unplaced::Refused(Rule::FlaggedPair))
    );
    // A held Bard spell over a non-Bard one, at effect 0: no conflict.
    let mut bard = effect(400, NORMAL, 0);
    bard.bard = Some(true);
    assert_eq!(
        over(bard, effect(401, NORMAL, 0)).map(|placed| placed.slot),
        Ok(3)
    );
    // Unknown Bard levels at those effects leave it unslotted.
    let mut unknown = effect(400, NORMAL, 0);
    unknown.bard = None;
    assert_eq!(
        over(unknown, effect(401, NORMAL, 0)),
        Err(Unplaced::SpellFieldsUnknown)
    );
}

#[test]
fn a_flagged_replace_only_nominates_and_lands_in_its_own_band() {
    let effect = |id, band| spell(id, band, &[(7, 5, 100)]);
    // Both flagged, the new stronger: the held slot is nominated, nothing
    // removed until after the loop, where the slot's other spell goes.
    let mut stronger = effect(401, BENEFICIAL);
    stronger.effects.as_mut().unwrap()[0].base = 9;
    assert_eq!(
        over(effect(400, BENEFICIAL), stronger),
        Ok(Placed {
            slot: 2,
            removed: Some(2),
            rule: Some(Rule::SongNominated),
        })
    );
    // A song replacing a normal-band buff lands in the song band, and
    // removes nothing there [I].
    let mut song = effect(701, SONG);
    song.effects.as_mut().unwrap()[0].base = 9;
    assert_eq!(
        over(effect(400, BENEFICIAL), song),
        Ok(Placed {
            slot: 25,
            removed: None,
            rule: Some(Rule::SongNominated),
        })
    );
}

#[test]
fn a_replace_outside_the_new_band_takes_the_band_s_first_empty_slot() {
    let speed = |id, band, base| spell(id, band, &[(11, base, 100)]);
    // A normal-band slow over a haste held in the song band.
    let slots = held(&[(25, speed(700, NORMAL, 130))]);
    assert_eq!(
        place(&slots, speed(401, NORMAL, 70)),
        Ok(Placed {
            slot: 0,
            removed: Some(25),
            rule: Some(Rule::Replaced),
        })
    );
}
