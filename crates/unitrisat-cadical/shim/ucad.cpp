// Project shim over the vendored CaDiCaL 2.2.1 for unitrisat-cadical.
//
// A C interface for Rust: whole clauses, assumptions, a solve with an
// interrupt hook, and model values. No C++ exception crosses into Rust:
// every entry point catches and reports failure. The Rust caller enforces
// CaDiCaL's state machine (no value outside the SATISFIED state, no zero or
// INT_MIN literal), so CaDiCaL's REQUIRE checks never abort.

#include "cadical.hpp"

#include <cstddef>
#include <memory>

namespace {

class Hook : public CaDiCaL::Terminator {
public:
  int (*callback) (void *) = nullptr;
  void *state = nullptr;
  bool terminate () override { return callback && callback (state) != 0; }
};

} // namespace

struct ucad {
  CaDiCaL::Solver solver;
  Hook hook;
};

extern "C" {

// A new solver with factor=0 set explicitly (the upstream default):
// bounded variable addition would introduce extension variables that a
// later input literal could collide with (REQUIRE_VALID_LIT, and Solver::add's
// declared-variable check). Returns null on failure.
ucad *ucad_new () {
  try {
    std::unique_ptr<ucad> s (new ucad ());
    if (!s->solver.set ("factor", 0))
      return nullptr;
    return s.release ();
  } catch (...) {
    return nullptr;
  }
}

void ucad_free (ucad *s) { delete s; }

// Adds the clause lits[0..n) and its terminating zero. Returns 0 on
// success, 1 if CaDiCaL threw (the solver is then unusable).
int ucad_add_clause (ucad *s, const int *lits, size_t n) {
  try {
    for (size_t i = 0; i < n; i++)
      s->solver.add (lits[i]);
    s->solver.add (0);
    return 0;
  } catch (...) {
    return 1;
  }
}

// Assumes lits[0..n) for the next solve and solves with the hook polled
// by CaDiCaL's terminator. Returns 10 (SAT), 20 (UNSAT), 0 (interrupted),
// or -1 if CaDiCaL threw. After a throw CaDiCaL may be left in an
// arbitrary state (for example SOLVING), where any further call, even
// disconnect_terminator, fails a REQUIRE and aborts. So a throw ends all
// calls into the instance: the hook is only cleared, stays connected to the
// instance it belongs to, and the Rust caller never touches the instance
// again.
int ucad_solve (ucad *s, const int *assumptions, size_t n,
                int (*callback) (void *), void *state) {
  int result = -1;
  try {
    for (size_t i = 0; i < n; i++)
      s->solver.assume (assumptions[i]);
    s->hook.callback = callback;
    s->hook.state = state;
    s->solver.connect_terminator (&s->hook);
    result = s->solver.solve ();
    s->solver.disconnect_terminator ();
  } catch (...) {
    result = -1;
  }
  s->hook.callback = nullptr;
  s->hook.state = nullptr;
  return result;
}

// The value of lit in the model: lit if true, -lit if false. Only in the
// SATISFIED state. Returns 0 if CaDiCaL threw.
int ucad_val (ucad *s, int lit) {
  try {
    return s->solver.val (lit);
  } catch (...) {
    return 0;
  }
}

// The largest variable index CaDiCaL has seen, or -1 if it threw.
int ucad_vars (ucad *s) {
  try {
    return s->solver.vars ();
  } catch (...) {
    return -1;
  }
}

} // extern "C"
