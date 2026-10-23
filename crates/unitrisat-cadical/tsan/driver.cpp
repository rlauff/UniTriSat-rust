// Concurrent construction/options lookup/solving, including before main.
#include "cadical.hpp"
#include <atomic>
#include <cstdio>
#include <thread>
#include <vector>

// Exercise constant initialization across translation units.
static CaDiCaL::Solver before_main;

int main () {
  if (before_main.get ("report") != 0 || !before_main.set ("quiet", 1))
    return 1;
  constexpr int threads = 8;
  std::atomic<int> ready{0};
  std::atomic<bool> go{false};
  std::atomic<int> wrong{0};
  std::vector<std::thread> workers;
  for (int t = 0; t < threads; ++t) {
    workers.emplace_back ([&] {
      ready.fetch_add (1);
      while (!go.load ())
        std::this_thread::yield ();
      for (int r = 0; r < 100; ++r) {
        CaDiCaL::Solver solver;
        if (!solver.set ("quiet", 1) || solver.get ("report") != 0 ||
            !solver.set ("seed", r) || solver.get ("seed") != r)
          wrong.fetch_add (1);
        solver.add (1);
        solver.add (2);
        solver.add (0);
        solver.assume (-1);
        if (solver.solve () != 10 || solver.val (2) != 2)
          wrong.fetch_add (1);
        solver.assume (-1);
        solver.assume (-2);
        if (solver.solve () != 20)
          wrong.fetch_add (1);
      }
    });
  }
  while (ready.load () != threads)
    std::this_thread::yield ();
  go.store (true);
  for (auto &worker : workers)
    worker.join ();
  std::printf ("wrong results: %d\n", wrong.load ());
  return wrong.load () != 0;
}
