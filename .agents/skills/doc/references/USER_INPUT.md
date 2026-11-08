# The owner's input

This is what the owner said in the founding conversation on 2026-10-24, in order, with the
instructions addressed to the model of that day removed. It is frozen: never edit it. It states
the intent, and `DECISIONS.md` states what was decided later. When the two disagree, raise the
difference with the owner. "The user" below means the person who owns the machine and the folders.

## 1. The idea

We are building something cool. We have a filetree on the machine, and this is how we go from
ideas to a done state.

The user dumps information as text, speech or a picture. For now only text. We organize that
into per-folder reminders. Each reminder has a title, a due date, context, a spec, a plan and
a success definition.

The user sets reminders either through the input interface or by hand per folder. Do not
worry about that part. A project that runs in parallel owns that infrastructure.

Then the user either does the tasks themselves, or presses a button: delegate to AI. That is
where the magic happens. AI iteration one defines the task specifically enough that there is
enough knowledge to do it. That means massaging the existing information and asking the user
when something is still missing. The agentic task definition has to be a deterministic flow.
Define the task. Collect missing information. Get the plan in place. Get the user's approval,
or run in auto mode. Launch the execution agent. Then run an evaluation agent that makes sure
the result is up to the spec. Then we notify the user of success.

On the agentic side, see code/maxdokukin/dot-agents. Each per-directory `.fabr-space/.agents`
behaves as a directory-level agent knowledge point. There is also a `~/.fabr-space` at the
root, which is special: it defines per-machine settings and procedures. Inside
`~/.fabr-space/.agents` we keep the global agentic material and preferences. That is the
agentic layer.

How the user talks to the agent: a Telegram chat.

Some tasks need personal context, such as emails where the sender is X or the date is
something. For the current demo purposes, mark this as a feature that is presented but
intentionally left out of the implementation.

What is not left out is the personal data bank. It is a place that stores the user's
information. The agent uses tools when executing tasks, such as
`personal-data-bank.fill(firstname)`, but the agent never sees the data in between. The data
sits encrypted somehow. There is a level of privacy we have to account for here. The product
is demo-ish, so not too crazy, maybe actually zero for now, but that is the idea behind the
data bank. Access to data fields is approved per task. If I want to buy something, the agent
requests my name, my address and my payment method. In the personal data settings we can set
fields to always allow or always ask. If the user wants their name and address accessible to
any agent, they set those fields to always allow. The fields are hierarchical: name.first,
name.last, address.line_1, address.line_2, address.zip. Access can be granted to the whole or
to part of a data type.

Last note: the agent preferences have to be editable by the user, in a GUI. We have global
rules, global preferences and per-project preferences. dot-agents does not fit precisely, but
well enough for the demo.

## 2. The learning layer and the monitoring view

Additional thoughts.

Per task, when "do with agent" is pressed, the planning agent tries to figure out and gather
the context across the current directory and past execution experiences. This is an
additional layer. In past executions the user gave us answers, so we must store those in
learned global user preferences, or maybe local ones. The idea is that I only need to tell the
agent something once. Next time it should say: in x you chose y, do you still want y? Three
options: yes, y one more time; always y; no, z. There is intelligent learning that happens
after the task was executed, and it is part of the task execution state machine.

We should be able to see all executing agents somewhere.

The application explodes.

## 3. Lifecycle, focus, and the diagram

On the early grilling: sure. The lifecycle is reminder, then task, then defined task, then task
in progress, then task succeeded or failed. On the other point I think you are overthinking it
for now.

Build me an interactive architecture diagram to build the big picture, rendered in the
browser, HTML perhaps. Focus: we are designing the architecture with all of these in mind.
Realistically, multiple features come later and are currently just placeholders that shape
the foundation. Our goal is to shape and pour the foundation concrete.

## 4. The name

We are renaming fabr to fabric.

## 5. Feature waves

I think that is good. Add a tab of the feature waves. Wave 1 is the foundation. Can we define
it yet?

## 6. Initial setup

Define the initial setup too. For now it is the default folder, the Anthropic API key and
model, and the Telegram connection. Only Sonnet and Haiku are available as models. The
Telegram connection is step by step: it tells you one step at a time. Note it in wave 1 as onboarding.

## 7. Where this lives

Store all these formal decisions and plans locally in code/maxdokukin/fabr/.agents, through
the xewe_doc skill. This conversation is the golden input that steers the product.

Main point: do not document the code. Document design and philosophy.
