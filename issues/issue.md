# issues
This document will be used for listing/tracking sudden obvious bugs and unexpected behavior decoupled from github issues style.

It is shipped with the project and goes on with the commit history line to describe some bugs fixed or not fixed at a point in a specific commit `HEAD` and when they were first discovered.

Developers of this codebase must ensure to report any observed issues in here, as this is a top priority issue box and every issues in here is seen as to be one of the next to cover `milestones`.

***Why this approach and not github issue*** — because this codebase is private in github. we'd make issues in github public for app users to report to and we want to decouple issues reported between users of the app and issues reported by developers of the app — we believe developers are too close to obvious issues in hand to fix — whilst users issues are more or improvement bugs or some bugs which are not obvious to need user experience to expose them.

***Note:*** Fixed issues aren't to be erased in here hence they're to be labeled `fixed/completed`

## The Table
> This table acts as table of content for issues reporters, they have to put there issues number here before they report it — the number is incremental.

| issues | status | comment |
|--------|--------|---------|
| [0001](#0001) | report | — |
| [0002](#0002) | report | — |

**issues** = incremental number of issue reported it is a link will take user direct to issue content
**status** = can be either report or fixed if done fixed — one can also use some tag to give a weight to issue eg. report(strong|weak etc.) — but that is what the comment field is for.
**comment** = the comment field is used to write some extra infos about the issue at a specific point — can be changed.



## 0001
**Observation:** when two devices start and both have already trusted each other they both simultaneously try to connect if auto-connect is turned on for both of them — sometimes one might win but sometimes both can stuck is the connecting state which delay/stops the connection.
This is a bug and should be fixed — both devices have to be able to know if the other has already requested for connecting hence one should stop connecting

## 0002
**Observation:** When the desktop app is opened they're two or three ways to stop it or shut-it-down. the close button on the reactUI is only used to hide the app to a tray. to close/quit/shutdown the app one has to ether close it through taskbar-close or through the tray menus where they can click `Quit`. the bug is shown in that second option(the tray `Quit`). it closes/shutdown the app but something is failing so the terminals show an error.
```
    [0909/155249.920:ERROR:ui\gfx\win\window_impl.cc:172] Failed to unregister class Chrome_WidgetWin_0. Error = 1412
```
This observation has been seen happening on windows desktops.